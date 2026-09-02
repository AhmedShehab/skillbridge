use std::collections::HashSet;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_yaml::Value;
use walkdir::WalkDir;

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

    pub const fn instructions_only() -> Self {
        Self {
            skills: false,
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
        Box::new(ZcodeAdapter),
        Box::new(ZedAdapter),
        Box::new(WindsurfAdapter),
        Box::new(ContinueAdapter),
        Box::new(AmazonQAdapter),
        Box::new(KiroAdapter),
        Box::new(QwenAdapter),
        Box::new(PiAdapter),
        Box::new(GooseAdapter),
        Box::new(CrushAdapter),
        Box::new(FactoryAdapter),
        Box::new(OpenHandsAdapter),
        Box::new(RooAdapter),
        Box::new(AmpAdapter),
        Box::new(KimiAdapter),
        Box::new(JunieAdapter),
        Box::new(VibeAdapter),
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
struct ZcodeAdapter;
struct ZedAdapter;
struct WindsurfAdapter;
struct ContinueAdapter;
struct AmazonQAdapter;
struct KiroAdapter;
struct QwenAdapter;
struct PiAdapter;
struct GooseAdapter;
struct CrushAdapter;
struct FactoryAdapter;
struct OpenHandsAdapter;
struct RooAdapter;
struct AmpAdapter;
struct KimiAdapter;
struct JunieAdapter;
struct VibeAdapter;

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
    let mut seen = HashSet::new();
    for path in paths {
        if path.is_symlink() || is_sensitive(path) {
            continue;
        }
        if path.is_file() {
            if let Some(item) = discover_instruction_file(agent, scope, path, kind)? {
                items.push(item);
            }
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        for entry in WalkDir::new(path).follow_links(false) {
            let entry = entry.with_context(|| format!("walking {}", path.display()))?;
            let child = entry.path();
            if entry.file_type().is_symlink()
                || !entry.file_type().is_file()
                || !seen.insert(child.to_path_buf())
            {
                continue;
            }
            if let Some(item) = discover_instruction_file(agent, scope, child, kind)? {
                items.push(item);
            }
        }
    }
    Ok(items)
}

fn discover_instruction_file(
    agent: &str,
    scope: ScopeKind,
    path: &Path,
    kind: ItemKind,
) -> Result<Option<DiscoveredItem>> {
    if !is_instruction_file(path) || is_generated_instruction(path) || is_sensitive(path) {
        return Ok(None);
    }
    let files = collect_files(path, kind)?;
    if files.is_empty() {
        return Ok(None);
    }
    Ok(Some(DiscoveredItem {
        agent: agent.to_string(),
        kind,
        scope,
        native_path: path.to_path_buf(),
        canonical_name: format!("{}-{}", agent, safe_file_name(path)),
        content_hash: hash_files(&files),
        files,
    }))
}

fn is_instruction_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if matches!(
        file_name,
        ".aider.conf"
            | ".clinerules"
            | ".cursorrules"
            | ".goosehints"
            | ".rules"
            | ".roorules"
            | ".windsurfrules"
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "mdc" | "txt")
    )
}

fn is_generated_instruction(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("agent-sync.md" | "agent-sync.mdc" | "skillbridge.md" | "skillbridge.mdc")
    )
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
                "---\ndescription: Shared instructions managed by SkillBridge\nglobs: []\nalwaysApply: true\n---\n\n",
            )
        } else {
            String::from(
                "<!-- Managed by SkillBridge. Edit canonical profile files instead. -->\n\n",
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
    ($type:ty, $id:literal, $name:literal, $summary:literal, $caps:expr, $commands:expr, $global_paths:expr, $project_paths:expr, $global_discovery:expr, $project_discovery:expr, $global_skills:expr, $project_skills:expr, $global_target:expr, $project_target:expr, $format:expr) => {
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
                    ScopeKind::Global => $global_discovery(context),
                    ScopeKind::Project => $project_discovery(context),
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
                    ScopeKind::Global => $global_target(context).into_iter().next(),
                    ScopeKind::Project => $project_target(context).into_iter().next(),
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

fn zed_agents_path(context: &HostContext) -> PathBuf {
    if cfg!(windows) {
        context.home.join("AppData/Roaming/Zed/AGENTS.md")
    } else {
        context.home.join(".config/zed/AGENTS.md")
    }
}

fn cline_global_rules_paths(context: &HostContext) -> Vec<PathBuf> {
    [
        context.home.join("Documents/Cline/Rules"),
        context.home.join("Cline/Rules"),
        context.home.join(".clinerules"),
    ]
    .into_iter()
    .collect()
}

fn cline_global_rules_target(context: &HostContext) -> PathBuf {
    let documents = context.home.join("Documents/Cline/Rules");
    let legacy = context.home.join("Cline/Rules");
    let directory = if documents.exists() || !legacy.exists() {
        documents
    } else {
        legacy
    };
    directory.join("agent-sync.md")
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
    |context: &HostContext| vec![context.home.join(".claude/CLAUDE.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("CLAUDE.md")],
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
    |context: &HostContext| vec![context.home.join(".codex/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
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
    |context: &HostContext| vec![context.home.join(".gemini/GEMINI.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("GEMINI.md")],
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
        &[
            ".cline",
            ".cline/skills",
            ".clinerules",
            "Documents/Cline/Rules",
            "Cline/Rules"
        ]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[
            ".clinerules",
            ".cline",
            ".cline/skills",
            ".cursorrules",
            ".windsurfrules",
            "AGENTS.md"
        ]
    ),
    |context: &HostContext| cline_global_rules_paths(context),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".clinerules", ".cursorrules", ".windsurfrules", "AGENTS.md"]
    ),
    [".cline/skills", ".agents/skills"],
    [
        ".cline/skills",
        ".clinerules/skills",
        ".claude/skills",
        ".agents/skills"
    ],
    |context: &HostContext| vec![cline_global_rules_target(context)],
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
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".cursor/rules", ".cursorrules", "AGENTS.md"]
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
    |context: &HostContext| vec![context.home.join(".copilot/copilot-instructions.md")],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[
            ".github/copilot-instructions.md",
            ".github/instructions",
            "AGENTS.md"
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
    |context: &HostContext| vec![context.home.join(".config/opencode/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    [
        ".config/opencode/skills",
        ".claude/skills",
        ".agents/skills"
    ],
    [".opencode/skills", ".claude/skills", ".agents/skills"],
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
    |context: &HostContext| vec![context.home.join("CONVENTIONS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("CONVENTIONS.md")],
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

adapter_impl!(
    ZcodeAdapter,
    "zcode",
    "ZCode",
    "AGENTS.md instructions",
    Capabilities::instructions_only(),
    ["zcode"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".zcode"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    |context: &HostContext| vec![context.home.join(".zcode/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    [],
    [],
    |context: &HostContext| vec![context.home.join(".zcode/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    ZedAdapter,
    "zed",
    "Zed Agent",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["zed"],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".config/zed", "AppData/Roaming/Zed"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[
            "AGENTS.md",
            "AGENT.md",
            ".rules",
            ".cursorrules",
            ".windsurfrules",
            ".clinerules",
            ".github/copilot-instructions.md",
            "CLAUDE.md",
            "GEMINI.md",
        ]
    ),
    |context: &HostContext| vec![zed_agents_path(context)],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[
            "AGENTS.md",
            "AGENT.md",
            ".rules",
            ".cursorrules",
            ".windsurfrules",
            ".clinerules",
            ".github/copilot-instructions.md",
            "CLAUDE.md",
            "GEMINI.md",
        ]
    ),
    [".agents/skills"],
    [".agents/skills"],
    |context: &HostContext| vec![zed_agents_path(context)],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    WindsurfAdapter,
    "windsurf",
    "Windsurf",
    "Cascade rules and AGENTS.md",
    Capabilities::instructions_only(),
    ["windsurf"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".codeium/windsurf"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".windsurf", ".windsurfrules", "AGENTS.md"]
    ),
    |context: &HostContext| vec![context
        .home
        .join(".codeium/windsurf/memories/global_rules.md")],
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".windsurf/rules", ".windsurfrules", "AGENTS.md"]
    ),
    [],
    [],
    |context: &HostContext| vec![context
        .home
        .join(".codeium/windsurf/memories/global_rules.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".windsurf/rules/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    ContinueAdapter,
    "continue",
    "Continue CLI",
    ".continue/rules",
    Capabilities::instructions_only(),
    ["cn", "continue"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".continue"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".continue"]),
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".continue/rules"]),
    [],
    [],
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".continue/rules/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    AmazonQAdapter,
    "amazon-q",
    "Amazon Q Developer",
    ".amazonq/rules",
    Capabilities::instructions_only(),
    ["q"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".amazonq", ".aws/amazonq"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".amazonq/rules"]),
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".amazonq/rules"]),
    [],
    [],
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".amazonq/rules/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    KiroAdapter,
    "kiro",
    "Kiro CLI",
    ".kiro/steering and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["kiro-cli", "kiro"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".kiro"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".kiro", "AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".kiro/steering"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".kiro/steering", "AGENTS.md"]
    ),
    [".kiro/skills", ".agents/skills"],
    [".kiro/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".kiro/steering/agent-sync.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".kiro/steering/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    QwenAdapter,
    "qwen",
    "Qwen Code",
    "QWEN.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["qwen", "qwen-code"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".qwen"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".qwen", "QWEN.md", "AGENTS.md"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".qwen/QWEN.md", ".qwen/AGENTS.md"]
    ),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["QWEN.md", "AGENTS.md"]),
    [".qwen/skills", ".agents/skills"],
    [".qwen/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".qwen/QWEN.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("QWEN.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    PiAdapter,
    "pi",
    "Pi",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["pi"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".pi/agent"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".pi", "AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".pi/agent/AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [".pi/agent/skills", ".agents/skills"],
    [".pi/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".pi/agent/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    GooseAdapter,
    "goose",
    "Goose",
    ".goosehints and AGENTS.md",
    Capabilities::instructions_only(),
    ["goose"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".config/goose"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".goosehints", "AGENTS.md"]),
    |context: &HostContext| vec![context.home.join(".config/goose/.goosehints")],
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".goosehints", "AGENTS.md"]),
    [],
    [],
    |context: &HostContext| vec![context.home.join(".config/goose/.goosehints")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".goosehints")],
    InstructionFormat::Markdown
);

adapter_impl!(
    CrushAdapter,
    "crush",
    "Crush",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["crush"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".config/crush"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".crush", "AGENTS.md"]),
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [
        ".config/crush/skills",
        ".config/agents/skills",
        ".agents/skills"
    ],
    [
        ".crush/skills",
        ".agents/skills",
        ".claude/skills",
        ".cursor/skills"
    ],
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    FactoryAdapter,
    "factory",
    "Factory Droid",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["droid"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".factory"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".factory", "AGENTS.md"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[
            ".factory/AGENTS.md",
            ".agents/AGENTS.md",
            ".agent/AGENTS.md"
        ]
    ),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [".factory/skills", ".agents/skills", ".agent/skills"],
    [".factory/skills", ".agents/skills", ".agent/skills"],
    |context: &HostContext| vec![context.home.join(".factory/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    OpenHandsAdapter,
    "openhands",
    "OpenHands",
    "Agent Skills and AGENTS.md",
    Capabilities::skills_and_instructions(),
    ["openhands"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".openhands"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".openhands", "AGENTS.md"]),
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [
        ".agents/skills",
        ".openhands/skills",
        ".openhands/microagents"
    ],
    [
        ".agents/skills",
        ".openhands/skills",
        ".openhands/microagents"
    ],
    |_context: &HostContext| Vec::new(),
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    RooAdapter,
    "roo",
    "Roo Code",
    "Roo rules and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["roo"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".roo"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".roo", ".roorules", "AGENTS.md"]
    ),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".roo/rules", ".roorules"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &[".roo/rules", ".roorules", "AGENTS.md"]
    ),
    [
        ".roo/skills",
        ".roo/skills-code",
        ".agents/skills",
        ".agents/skills-code"
    ],
    [
        ".roo/skills",
        ".roo/skills-code",
        ".agents/skills",
        ".agents/skills-code"
    ],
    |context: &HostContext| vec![context.home.join(".roo/rules/agent-sync.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join(".roo/rules/agent-sync.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    AmpAdapter,
    "amp",
    "Amp",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["amp"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".config/amp"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md", "AGENT.md"]),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Global,
        &[".config/amp/AGENTS.md", ".config/AGENTS.md"]
    ),
    |context: &HostContext| path_list(
        context,
        ScopeKind::Project,
        &["AGENTS.md", "AGENT.md", "CLAUDE.md"]
    ),
    [
        ".config/agents/skills",
        ".agents/skills",
        ".config/amp/skills"
    ],
    [".agents/skills", ".claude/skills"],
    |context: &HostContext| vec![context.home.join(".config/amp/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    KimiAdapter,
    "kimi",
    "Kimi Code CLI",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["kimi", "kimi-cli"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".kimi"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".kimi", "AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".kimi/AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [".kimi/skills", ".config/agents/skills", ".agents/skills"],
    [".kimi/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".kimi/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    JunieAdapter,
    "junie",
    "Junie",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["junie"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".junie"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".junie", "AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".junie/AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [".junie/skills", ".agents/skills"],
    [".junie/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".junie/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);

adapter_impl!(
    VibeAdapter,
    "vibe",
    "Mistral Vibe",
    "AGENTS.md and Agent Skills",
    Capabilities::skills_and_instructions(),
    ["vibe"],
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".vibe"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &[".vibe", "AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Global, &[".vibe/AGENTS.md"]),
    |context: &HostContext| path_list(context, ScopeKind::Project, &["AGENTS.md"]),
    [".vibe/skills", ".agents/skills"],
    [".vibe/skills", ".agents/skills"],
    |context: &HostContext| vec![context.home.join(".vibe/AGENTS.md")],
    |context: &HostContext| vec![context
        .project_root
        .clone()
        .unwrap_or_default()
        .join("AGENTS.md")],
    InstructionFormat::Markdown
);
