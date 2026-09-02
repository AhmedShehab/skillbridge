use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::Value;

use crate::model::{
    CanonicalItem, DiscoveredItem, FilePayload, HostContext, ItemKind, Materialization, ScopeKind,
};
use crate::safety::{collect_files, hash_files, is_sensitive};

#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub skills: bool,
    pub instructions: bool,
    pub memory: bool,
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "skills={},instructions={},memory={}",
            self.skills, self.instructions, self.memory
        )
    }
}

impl Capabilities {
    pub const fn skills_and_instructions() -> Self {
        Self {
            skills: true,
            instructions: true,
            memory: false,
        }
    }
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn summary(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn detect(&self, context: &HostContext) -> bool;
    fn discover(&self, context: &HostContext, scope: ScopeKind) -> Result<Vec<DiscoveredItem>>;
    fn render(
        &self,
        context: &HostContext,
        scope: ScopeKind,
        items: &[CanonicalItem],
    ) -> Result<Vec<Materialization>>;
}

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn AgentAdapter>>,
}

impl AdapterRegistry {
    pub fn new(adapters: Vec<Box<dyn AgentAdapter>>) -> Self {
        Self { adapters }
    }

    pub fn all(&self) -> &[Box<dyn AgentAdapter>] {
        &self.adapters
    }

    pub fn select(&self, requested: Option<&str>) -> Vec<&dyn AgentAdapter> {
        self.adapters
            .iter()
            .filter(|adapter| requested.is_none_or(|name| adapter.id() == name))
            .map(std::convert::AsRef::as_ref)
            .collect()
    }
}

pub fn registry() -> AdapterRegistry {
    AdapterRegistry::new(vec![
        Box::new(ClaudeAdapter),
        Box::new(CodexAdapter),
        Box::new(GeminiAdapter),
        Box::new(ClineAdapter),
        Box::new(CursorAdapter),
        Box::new(CopilotAdapter),
        Box::new(OpenCodeAdapter),
        Box::new(AiderAdapter),
    ])
}

struct ClaudeAdapter;
struct CodexAdapter;
struct GeminiAdapter;
struct ClineAdapter;
struct CursorAdapter;
struct CopilotAdapter;
struct OpenCodeAdapter;
struct AiderAdapter;

fn detect_any(_context: &HostContext, paths: &[PathBuf], commands: &[&str]) -> bool {
    paths.iter().any(|path| path.exists())
        || commands.iter().any(|command| command_on_path(command))
}

fn command_on_path(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            let candidate = directory.join(command);
            candidate.is_file()
                || (cfg!(windows) && directory.join(format!("{command}.exe")).is_file())
        })
    })
}

fn scope_root(context: &HostContext, scope: ScopeKind) -> Result<PathBuf> {
    match scope {
        ScopeKind::Global => Ok(context.home.clone()),
        ScopeKind::Project => context
            .project_root
            .clone()
            .context("project scope requires a project root; pass --project"),
    }
}

fn discover_skills(
    agent: &str,
    _context: &HostContext,
    scope: ScopeKind,
    roots: &[PathBuf],
) -> Result<Vec<DiscoveredItem>> {
    let mut items = Vec::new();
    for root in roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() || path.is_symlink() || is_sensitive(&path) {
                continue;
            }
            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }
            let files = collect_files(&path, ItemKind::Skill)?;
            validate_skill(&skill_file)?;
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("unnamed")
                .to_string();
            items.push(DiscoveredItem {
                agent: agent.to_string(),
                kind: ItemKind::Skill,
                scope,
                native_path: path,
                canonical_name: name,
                content_hash: hash_files(&files),
                files,
            });
        }
    }
    Ok(items)
}

fn discover_files(
    agent: &str,
    _context: &HostContext,
    scope: ScopeKind,
    paths: &[PathBuf],
    kind: ItemKind,
) -> Result<Vec<DiscoveredItem>> {
    let mut items = Vec::new();
    for path in paths {
        if path.is_file() && !is_sensitive(path) {
            let files = collect_files(path, kind)?;
            if !files.is_empty() {
                items.push(DiscoveredItem {
                    agent: agent.to_string(),
                    kind,
                    scope,
                    native_path: path.clone(),
                    canonical_name: format!("{}-{}", agent, safe_file_name(path)),
                    content_hash: hash_files(&files),
                    files,
                });
            }
        } else if path.is_dir() && !is_sensitive(path) {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let child = entry.path();
                if child.is_file() && !is_sensitive(&child) {
                    let files = collect_files(&child, kind)?;
                    if !files.is_empty() {
                        items.push(DiscoveredItem {
                            agent: agent.to_string(),
                            kind,
                            scope,
                            native_path: child.clone(),
                            canonical_name: format!("{}-{}", agent, safe_file_name(&child)),
                            content_hash: hash_files(&files),
                            files,
                        });
                    }
                }
            }
        }
    }
    Ok(items)
}

fn safe_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("instruction.md")
        .replace(['/', '\\'], "-")
}

fn skill_roots(context: &HostContext, scope: ScopeKind, relative: &[&str]) -> Result<Vec<PathBuf>> {
    let root = scope_root(context, scope)?;
    Ok(relative.iter().map(|path| root.join(path)).collect())
}

fn item_files(item: &CanonicalItem) -> Vec<FilePayload> {
    item.files.clone()
}

fn render_skills(
    agent: &str,
    _context: &HostContext,
    scope: ScopeKind,
    target_root: &Path,
    items: &[CanonicalItem],
) -> Vec<Materialization> {
    items
        .iter()
        .filter(|item| item.kind == ItemKind::Skill && item.scope == scope)
        .map(|item| Materialization {
            agent: agent.to_string(),
            kind: ItemKind::Skill,
            scope,
            item_id: item.id.clone(),
            canonical_hash: item.content_hash.clone(),
            target_root: target_root.join(skill_name(item)),
            files: item_files(item),
            replace_existing: true,
        })
        .collect::<Vec<_>>()
        .tap(|_| {})
}

fn skill_name(item: &CanonicalItem) -> String {
    item.relative_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("skill")
        .to_string()
}

fn render_instructions(
    agent: &str,
    _context: &HostContext,
    scope: ScopeKind,
    target: PathBuf,
    items: &[CanonicalItem],
    format: InstructionFormat,
) -> Vec<Materialization> {
    let relevant = items
        .iter()
        .filter(|item| {
            item.scope == scope && matches!(item.kind, ItemKind::Instruction | ItemKind::Memory)
        })
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return Vec::new();
    }
    let bytes = if format == InstructionFormat::Markdown && relevant.len() == 1 {
        relevant[0]
            .files
            .first()
            .map_or_else(Vec::new, |file| file.bytes.clone())
    } else {
        let mut body = if format == InstructionFormat::Mdc {
            String::from(
                "---\ndescription: Shared instructions managed by agent-sync\nglobs: []\nalwaysApply: true\n---\n\n",
            )
        } else {
            String::from(
                "<!-- Managed by agent-sync. Edit canonical profile files instead. -->\n\n",
            )
        };
        body.push_str("# Shared agent instructions\n\n");
        for item in relevant {
            let _ = writeln!(body, "## {}\n", item.id);
            if let Some(file) = item.files.first() {
                body.push_str(&String::from_utf8_lossy(&file.bytes));
            }
            body.push_str("\n\n");
        }
        body.into_bytes()
    };
    let files = vec![FilePayload {
        relative_path: target
            .file_name()
            .map_or_else(|| PathBuf::from("instructions.md"), PathBuf::from),
        bytes,
    }];
    let item_id = format!("instructions/{agent}/{}", target.display());
    vec![Materialization {
        agent: agent.to_string(),
        kind: ItemKind::Instruction,
        scope,
        item_id,
        canonical_hash: hash_files(&files),
        target_root: target,
        files,
        replace_existing: true,
    }]
}

fn validate_skill(path: &Path) -> Result<()> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let Some(rest) = text.strip_prefix("---") else {
        anyhow::bail!("{} is missing YAML frontmatter", path.display());
    };
    let Some((frontmatter, _body)) = rest.split_once("\n---") else {
        anyhow::bail!(
            "{} has an unterminated YAML frontmatter block",
            path.display()
        );
    };
    let value: Value = serde_yaml::from_str(frontmatter)
        .with_context(|| format!("invalid YAML frontmatter in {}", path.display()))?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() || description.is_empty() {
        anyhow::bail!(
            "{} must define non-empty name and description",
            path.display()
        );
    }
    let directory_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if name != directory_name {
        anyhow::bail!("{} name must match its skill directory", path.display());
    }
    if name.len() > 64
        || description.len() > 1024
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        anyhow::bail!("{} has an invalid Agent Skills name", path.display());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstructionFormat {
    Markdown,
    Mdc,
}

macro_rules! adapter_impl {
    ($type:ty, $id:literal, $name:literal, $summary:literal, $caps:expr, $commands:expr, $global_paths:expr, $project_paths:expr, $global_skills:expr, $project_skills:expr, $global_instruction:expr, $project_instruction:expr, $format:expr) => {
        impl AgentAdapter for $type {
            fn id(&self) -> &'static str {
                $id
            }
            fn name(&self) -> &'static str {
                $name
            }
            fn summary(&self) -> &'static str {
                $summary
            }
            fn capabilities(&self) -> Capabilities {
                $caps
            }
            fn detect(&self, context: &HostContext) -> bool {
                detect_any(context, &$global_paths(context), &$commands)
            }
            fn discover(
                &self,
                context: &HostContext,
                scope: ScopeKind,
            ) -> Result<Vec<DiscoveredItem>> {
                let skills = match scope {
                    ScopeKind::Global => discover_skills(
                        $id,
                        context,
                        scope,
                        &skill_roots(context, scope, &$global_skills)?,
                    )?,
                    ScopeKind::Project => discover_skills(
                        $id,
                        context,
                        scope,
                        &skill_roots(context, scope, &$project_skills)?,
                    )?,
                };
                let instruction_paths = match scope {
                    ScopeKind::Global => $global_instruction(context),
                    ScopeKind::Project => $project_instruction(context),
                };
                let mut files = discover_files(
                    $id,
                    context,
                    scope,
                    &instruction_paths,
                    ItemKind::Instruction,
                )?;
                let mut result = skills;
                result.append(&mut files);
                Ok(result)
            }
            fn render(
                &self,
                context: &HostContext,
                scope: ScopeKind,
                items: &[CanonicalItem],
            ) -> Result<Vec<Materialization>> {
                let skill_root = match scope {
                    ScopeKind::Global => skill_roots(context, scope, &$global_skills)?
                        .into_iter()
                        .next(),
                    ScopeKind::Project => skill_roots(context, scope, &$project_skills)?
                        .into_iter()
                        .next(),
                };
                let mut result = Vec::new();
                if let Some(root) = skill_root {
                    result.extend(render_skills($id, context, scope, &root, items));
                }
                let instruction_path = match scope {
                    ScopeKind::Global => $global_instruction(context).into_iter().next(),
                    ScopeKind::Project => $project_instruction(context).into_iter().next(),
                };
                if let Some(path) = instruction_path {
                    result.extend(render_instructions(
                        $id, context, scope, path, items, $format,
                    ));
                }
                Ok(result)
            }
        }
    };
}

fn path_list(context: &HostContext, scope: ScopeKind, paths: &[&str]) -> Vec<PathBuf> {
    let root = scope_root(context, scope).unwrap_or_default();
    paths.iter().map(|path| root.join(path)).collect()
}

adapter_impl!(
    ClaudeAdapter,
    "claude",
    "Claude Code",
    "CLAUDE.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["claude"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".claude", ".claude/skills", ".claude/CLAUDE.md"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &["CLAUDE.md", ".claude", ".claude/skills"]
    ),
    [".claude/skills"],
    [".claude/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".claude/CLAUDE.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("CLAUDE.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    CodexAdapter,
    "codex",
    "OpenAI Codex CLI",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["codex"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".codex", ".agents", ".codex/AGENTS.md"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &["AGENTS.md", ".agents", ".codex"]
    ),
    [".codex/skills", ".agents/skills"],
    [".agents/skills", ".codex/skills"],
    |context: &HostContext| vec![context.home.join(".codex/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    GeminiAdapter,
    "gemini",
    "Gemini CLI",
    "GEMINI.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["gemini"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".gemini", ".gemini/skills", ".gemini/GEMINI.md"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &["GEMINI.md", ".gemini", ".gemini/skills"]
    ),
    [".gemini/skills", ".agents/skills"],
    [".gemini/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".gemini/GEMINI.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("GEMINI.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    ClineAdapter,
    "cline",
    "Cline",
    ".clinerules and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["cline"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".cline", ".cline/skills", ".clinerules"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".clinerules", ".cline", ".cline/skills"]
    ),
    [".cline/skills", ".agents/skills"],
    [".cline/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".clinerules/agent-sync.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".clinerules/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    CursorAdapter,
    "cursor",
    "Cursor",
    ".cursor/rules and supported Agent Skills",
    Capabilities::skills_and_instructions(),
    ["cursor"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".cursor", ".cursor/skills"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".cursor", ".cursor/rules", ".cursor/skills"]
    ),
    [".cursor/skills", ".agents/skills"],
    [".cursor/skills", ".agents/skills"],
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".cursor/rules/agent-sync.mdc")],
    InstructionFormat::Mdc
);

adapter_impl!(
    CopilotAdapter,
    "copilot",
    "GitHub Copilot CLI",
    ".github/copilot-instructions.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["copilot"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[
            ".copilot",
            ".copilot/skills",
            ".copilot/copilot-instructions.md"
        ]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[
            ".github",
            ".github/skills",
            ".github/copilot-instructions.md"
        ]
    ),
    [".copilot/skills", ".agents/skills"],
    [".github/skills", ".agents/skills", ".claude/skills"],
    |context: &HostContext| vec![context.home.join(".copilot/copilot-instructions.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".github/copilot-instructions.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    OpenCodeAdapter,
    "opencode",
    "OpenCode",
    "Agent Skills and AGENTS.md",
    Capabilities::skills_and_instructions(),
    ["opencode"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".config/opencode", ".config/opencode/skills"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".opencode", ".opencode/skills", "AGENTS.md"]
    ),
    [".config/opencode/skills", ".agents/skills"],
    [".opencode/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".config/opencode/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    AiderAdapter,
    "aider",
    "Aider",
    "Conventions files and explicit read files",
    Capabilities {
        skills: false,
        instructions: true,
        memory: false
    },
    ["aider"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[
            ".aider.conf.yml",
            ".aider.conf.yaml",
            ".aider",
            "CONVENTIONS.md",
        ]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".aider.conf.yml", ".aider.conf.yaml", "CONVENTIONS.md"]
    ),
    [],
    [],
    |context: &HostContext| vec![context.home.join("CONVENTIONS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("CONVENTIONS.md")],
    InstructionFormat::Markdown
);

trait Tap: Sized {
    fn tap<F: FnOnce(&Self)>(self, function: F) -> Self {
        function(&self);
        self
    }
}

impl<T> Tap for T {}
