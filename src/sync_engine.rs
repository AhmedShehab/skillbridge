use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::adapters::{AdapterRegistry, AgentAdapter};
use crate::cli::{ResolveArg, ScopeArg};
use crate::model::{
    ChangeKind, Diagnostic, DiagnosticLevel, DiscoveredItem, HostContext, ManifestEntry,
    Materialization, ScopeKind, SyncPlan,
};
use crate::safety::{
    collect_files, ensure_inside, has_symlink_component, hash_files, is_sensitive,
    validate_relative_path,
};
use crate::store::{atomic_write, CanonicalStore};

pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub conflicts: usize,
}

pub struct ApplyResult {
    pub applied: usize,
    pub skipped: usize,
}

pub fn scopes(scope: ScopeArg) -> Vec<ScopeKind> {
    match scope {
        ScopeArg::Global => vec![ScopeKind::Global],
        ScopeArg::Project => vec![ScopeKind::Project],
        ScopeArg::Both => vec![ScopeKind::Global, ScopeKind::Project],
    }
}

pub fn scan(
    adapters: &[&dyn AgentAdapter],
    context: &HostContext,
    scope: ScopeArg,
) -> Result<Vec<DiscoveredItem>> {
    let mut result = Vec::new();
    for adapter in adapters {
        for scope in scopes(scope) {
            if scope == ScopeKind::Project && context.project_root.is_none() {
                continue;
            }
            result.extend(adapter.discover(context, scope)?);
        }
    }
    result.sort_by(|left, right| {
        left.agent
            .cmp(&right.agent)
            .then(left.scope.to_string().cmp(&right.scope.to_string()))
            .then(left.native_path.cmp(&right.native_path))
    });
    Ok(result)
}

pub fn import(
    store: &CanonicalStore,
    adapters: &[&dyn AgentAdapter],
    context: &HostContext,
    scope: ScopeArg,
) -> Result<ImportResult> {
    let discovered = scan(adapters, context, scope)?;
    let project_id = project_id_for_store(store, context);
    let mut result = ImportResult {
        imported: 0,
        skipped: 0,
        conflicts: 0,
    };
    for item in discovered {
        let name = canonical_import_name(&item);
        match store.import_item(
            item.scope,
            project_id.as_deref(),
            item.kind,
            &name,
            &item.files,
        ) {
            Ok(true) => result.imported += 1,
            Ok(false) => result.skipped += 1,
            Err(error) => {
                result.conflicts += 1;
                eprintln!("conflict: {} ({})", item.native_path.display(), error);
            }
        }
    }
    Ok(result)
}

fn canonical_import_name(item: &DiscoveredItem) -> String {
    match item.kind {
        crate::model::ItemKind::Skill => item.canonical_name.clone(),
        crate::model::ItemKind::Instruction | crate::model::ItemKind::Memory => {
            format!("{}.md", item.canonical_name.trim_end_matches(".md"))
        }
    }
}

pub fn plan(
    store: &CanonicalStore,
    adapters: &[&dyn AgentAdapter],
    context: &HostContext,
    scope: ScopeArg,
) -> Result<SyncPlan> {
    let project_id = project_id_for_store(store, context);
    let mut plan = SyncPlan::default();
    let mut active_keys = HashSet::new();
    for adapter in adapters {
        for scope in scopes(scope) {
            if scope == ScopeKind::Project && context.project_root.is_none() {
                continue;
            }
            let items = store.read_items(scope, project_id.as_deref())?;
            let materializations = adapter.render(context, scope, &items)?;
            for materialization in materializations {
                active_keys.insert(manifest_key(&materialization));
                plan.changes
                    .push(classify_materialization(store, materialization)?);
            }
        }
    }
    add_stale_manifest_entries(store, adapters, context, scope, &active_keys, &mut plan)?;
    plan.changes.sort_by(|left, right| {
        left.agent
            .cmp(&right.agent)
            .then(left.target_root.cmp(&right.target_root))
    });
    Ok(plan)
}

fn add_stale_manifest_entries(
    store: &CanonicalStore,
    adapters: &[&dyn AgentAdapter],
    context: &HostContext,
    scope: ScopeArg,
    active_keys: &HashSet<String>,
    plan: &mut SyncPlan,
) -> Result<()> {
    let manifest = store.manifest()?;
    let selected_agents = adapters
        .iter()
        .map(|adapter| adapter.id())
        .collect::<HashSet<_>>();
    let selected_scopes = scopes(scope).into_iter().collect::<HashSet<_>>();
    for entry in manifest.entries {
        if !selected_agents.contains(entry.agent.as_str())
            || !selected_scopes.contains(&entry.scope)
            || (entry.scope == ScopeKind::Project && context.project_root.is_none())
        {
            continue;
        }
        let key = manifest_key_from_entry(&entry);
        if !active_keys.contains(&key) {
            plan.changes.push(crate::model::PlannedChange {
                kind: ChangeKind::Delete,
                agent: entry.agent,
                scope: entry.scope,
                item_id: entry.item_id,
                target_root: PathBuf::from(entry.target_root),
                canonical_hash: None,
                target_hash: Some(entry.target_hash),
                materialization: None,
                message: "canonical item was removed; deletion is not automatic".to_string(),
            });
        }
    }
    Ok(())
}

fn classify_materialization(
    store: &CanonicalStore,
    materialization: Materialization,
) -> Result<crate::model::PlannedChange> {
    let target_hash = target_hash(&materialization)?;
    let manifest = store.manifest()?;
    let key = manifest_key(&materialization);
    let previous = manifest
        .entries
        .iter()
        .find(|entry| manifest_key_from_entry(entry) == key);
    let kind = match (&target_hash, previous) {
        (None, _) => ChangeKind::Create,
        (Some(hash), None) if hash == &materialization.canonical_hash => ChangeKind::Update,
        (Some(hash), Some(previous)) if hash == &materialization.canonical_hash => {
            if previous.canonical_hash == materialization.canonical_hash
                && previous.target_hash == *hash
            {
                ChangeKind::Skip
            } else {
                ChangeKind::Update
            }
        }
        (Some(hash), Some(previous)) if hash == &previous.target_hash => {
            if previous.canonical_hash == materialization.canonical_hash {
                ChangeKind::Skip
            } else {
                ChangeKind::Update
            }
        }
        (Some(_), Some(previous)) if previous.canonical_hash == materialization.canonical_hash => {
            ChangeKind::Conflict
        }
        (Some(_), None) | (Some(_), Some(_)) => ChangeKind::Conflict,
    };
    let message = match kind {
        ChangeKind::Create => "target does not exist".to_string(),
        ChangeKind::Update => "canonical content changed".to_string(),
        ChangeKind::Conflict => {
            "canonical and target both changed, or target is unmanaged".to_string()
        }
        ChangeKind::Skip => "up to date".to_string(),
        ChangeKind::Delete => "canonical item was removed; deletion is not automatic".to_string(),
    };
    Ok(crate::model::PlannedChange {
        kind,
        agent: materialization.agent.clone(),
        scope: materialization.scope,
        item_id: materialization.item_id.clone(),
        target_root: materialization.target_root.clone(),
        canonical_hash: Some(materialization.canonical_hash.clone()),
        target_hash,
        materialization: Some(materialization),
        message,
    })
}

fn target_hash(materialization: &Materialization) -> Result<Option<String>> {
    if materialization.kind == crate::model::ItemKind::Skill {
        if !materialization.target_root.exists() {
            return Ok(None);
        }
        let files = collect_files(&materialization.target_root, materialization.kind)?;
        return Ok((!files.is_empty()).then(|| hash_files(&files)));
    }
    if !materialization.target_root.exists() {
        return Ok(None);
    }
    let files = collect_files(&materialization.target_root, materialization.kind)?;
    Ok((!files.is_empty()).then(|| hash_files(&files)))
}

pub fn apply(
    store: &CanonicalStore,
    plan: &SyncPlan,
    resolve: Option<ResolveArg>,
    project_id: Option<&str>,
) -> Result<ApplyResult> {
    let mut applied = 0;
    let mut skipped = 0;
    let mut manifest = store.manifest()?;
    for change in &plan.changes {
        if change.kind == ChangeKind::Skip {
            skipped += 1;
            continue;
        }
        let Some(materialization) = &change.materialization else {
            skipped += 1;
            continue;
        };
        if change.kind == ChangeKind::Conflict {
            match resolve {
                Some(ResolveArg::Canonical) => {}
                Some(ResolveArg::Target) => {
                    import_target_to_canonical(store, materialization, project_id)?;
                    skipped += 1;
                    continue;
                }
                None => anyhow::bail!("unresolved conflict: {}", change.target_root.display()),
            }
        }
        write_materialization(materialization)?;
        let target_hash = target_hash(materialization)?.unwrap_or_default();
        manifest
            .entries
            .retain(|entry| manifest_key_from_entry(entry) != manifest_key(materialization));
        manifest.entries.push(ManifestEntry {
            agent: materialization.agent.clone(),
            kind: materialization.kind,
            scope: materialization.scope,
            item_id: materialization.item_id.clone(),
            target_root: materialization.target_root.to_string_lossy().into_owned(),
            canonical_hash: materialization.canonical_hash.clone(),
            target_hash,
        });
        applied += 1;
    }
    store.save_manifest(&manifest)?;
    Ok(ApplyResult { applied, skipped })
}

fn write_materialization(materialization: &Materialization) -> Result<()> {
    if materialization.target_root.exists() && !materialization.replace_existing {
        anyhow::bail!(
            "refusing to replace existing target: {}",
            materialization.target_root.display()
        );
    }
    let allowed_root = if materialization.kind == crate::model::ItemKind::Skill {
        materialization
            .target_root
            .parent()
            .context("skill target has no parent")?
    } else {
        materialization
            .target_root
            .parent()
            .context("file target has no parent")?
    };
    if allowed_root.exists() {
        ensure_inside(allowed_root, &materialization.target_root)?;
    } else if let Some(parent) = allowed_root.parent() {
        if parent.exists() {
            ensure_inside(parent, allowed_root)?;
        }
    }
    if has_symlink_component(allowed_root, &materialization.target_root) {
        anyhow::bail!(
            "refusing to write through symlinked target: {}",
            materialization.target_root.display()
        );
    }
    if materialization.kind == crate::model::ItemKind::Skill {
        fs::create_dir_all(&materialization.target_root)?;
        for file in &materialization.files {
            validate_relative_path(&file.relative_path)?;
            let path = materialization.target_root.join(&file.relative_path);
            if has_symlink_component(&materialization.target_root, &path) {
                anyhow::bail!(
                    "refusing to write through symlinked target: {}",
                    path.display()
                );
            }
            if is_sensitive(&path) {
                anyhow::bail!("refusing to materialize sensitive path: {}", path.display());
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&path, &file.bytes)?;
        }
        remove_stale_skill_files(materialization)?;
    } else if let Some(file) = materialization.files.first() {
        if is_sensitive(&materialization.target_root) {
            anyhow::bail!(
                "refusing to materialize sensitive path: {}",
                materialization.target_root.display()
            );
        }
        atomic_write(&materialization.target_root, &file.bytes)?;
    }
    Ok(())
}

fn remove_stale_skill_files(materialization: &Materialization) -> Result<()> {
    let expected = materialization
        .files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<HashSet<_>>();
    let mut directories = Vec::new();
    for entry in WalkDir::new(&materialization.target_root)
        .follow_links(false)
        .contents_first(true)
    {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type().is_symlink() {
            anyhow::bail!("refusing to clean through symlink: {}", path.display());
        }
        if entry.file_type().is_dir() {
            directories.push(path.to_path_buf());
            continue;
        }
        if !entry.file_type().is_file() || is_sensitive(path) {
            continue;
        }
        let relative = path.strip_prefix(&materialization.target_root)?;
        if !expected.contains(relative) {
            fs::remove_file(path)
                .with_context(|| format!("removing stale generated file {}", path.display()))?;
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if directory != materialization.target_root {
            let _ = fs::remove_dir(directory);
        }
    }
    Ok(())
}

fn import_target_to_canonical(
    store: &CanonicalStore,
    materialization: &Materialization,
    project_id: Option<&str>,
) -> Result<()> {
    let Some(target_hash) = target_hash(materialization)? else {
        anyhow::bail!(
            "conflict target disappeared: {}",
            materialization.target_root.display()
        );
    };
    let files = collect_files(&materialization.target_root, materialization.kind)?;
    let name = materialization
        .item_id
        .split('/')
        .next_back()
        .unwrap_or("target.md");
    let name = if materialization.kind == crate::model::ItemKind::Skill {
        name.to_string()
    } else {
        format!("target-{name}.md")
    };
    store.import_item(
        materialization.scope,
        project_id,
        materialization.kind,
        &name,
        &files,
    )?;
    println!("Imported target changes (hash {target_hash}) into canonical storage.");
    Ok(())
}

pub fn print_plan(plan: &SyncPlan) {
    if plan.changes.is_empty() {
        println!("No applicable adapter outputs.");
        return;
    }
    for change in &plan.changes {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\tcanonical={}\ttarget={}",
            change.kind,
            change.agent,
            change.scope,
            change.item_id,
            change.target_root.display(),
            change.message,
            change.canonical_hash.as_deref().unwrap_or("-"),
            change.target_hash.as_deref().unwrap_or("-")
        );
    }
}

pub fn print_status(plan: &SyncPlan) {
    let mut counts: HashMap<ChangeKind, usize> = HashMap::new();
    for change in &plan.changes {
        *counts.entry(change.kind).or_default() += 1;
    }
    print_plan(plan);
    println!(
        "summary: {} create, {} update, {} conflict, {} delete-pending, {} up-to-date",
        counts.get(&ChangeKind::Create).copied().unwrap_or_default(),
        counts.get(&ChangeKind::Update).copied().unwrap_or_default(),
        counts
            .get(&ChangeKind::Conflict)
            .copied()
            .unwrap_or_default(),
        counts.get(&ChangeKind::Delete).copied().unwrap_or_default(),
        counts.get(&ChangeKind::Skip).copied().unwrap_or_default(),
    );
}

pub fn doctor(
    store: &CanonicalStore,
    registry: &AdapterRegistry,
    context: &HostContext,
) -> Result<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    diagnostics.push(Diagnostic {
        level: DiagnosticLevel::Info,
        message: format!("profile: {}", store.root().display()),
    });
    let config = store.config()?;
    if config.schema_version != 1 {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            message: format!(
                "unsupported profile schema version {}",
                config.schema_version
            ),
        });
    }
    for adapter in registry.all() {
        let detected = adapter.detect(context);
        diagnostics.push(Diagnostic {
            level: if detected {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warning
            },
            message: format!(
                "adapter {}: {} ({})",
                adapter.id(),
                if detected { "detected" } else { "not detected" },
                adapter.capabilities()
            ),
        });
    }
    for scope in [ScopeKind::Global, ScopeKind::Project] {
        let project_id = project_id_for_store(store, context);
        for item in store.read_items(scope, project_id.as_deref())? {
            if item.kind == crate::model::ItemKind::Skill {
                let skill_file = item
                    .files
                    .iter()
                    .find(|file| file.relative_path == Path::new("SKILL.md"));
                if skill_file.is_none() {
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("skill {} is missing SKILL.md", item.id),
                    });
                }
            }
        }
    }
    Ok(diagnostics)
}

fn manifest_key(materialization: &Materialization) -> String {
    format!(
        "{}|{}|{}|{}",
        materialization.agent, materialization.kind, materialization.scope, materialization.item_id
    )
}

fn manifest_key_from_entry(entry: &ManifestEntry) -> String {
    format!(
        "{}|{}|{}|{}",
        entry.agent, entry.kind, entry.scope, entry.item_id
    )
}

pub fn project_id_for_context(context: &HostContext) -> Option<String> {
    let root = context.project_root.as_ref()?;
    let git_root = find_git_root(root).unwrap_or_else(|| root.clone());
    if let Ok(remote) = std::process::Command::new("git")
        .args([
            "-C",
            git_root.to_string_lossy().as_ref(),
            "config",
            "--get",
            "remote.origin.url",
        ])
        .output()
    {
        if remote.status.success() {
            let value = String::from_utf8_lossy(&remote.stdout).trim().to_string();
            if !value.is_empty() {
                return Some(slug(&value));
            }
        }
    }
    Some(slug(&git_root.to_string_lossy()))
}

pub fn project_id_for_store(store: &CanonicalStore, context: &HostContext) -> Option<String> {
    let current_root = context.project_root.as_ref()?.canonicalize().ok()?;
    if let Ok(config) = store.config() {
        for project in config.projects {
            let configured_root_value = project.root.clone();
            let configured_root = PathBuf::from(&configured_root_value)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(configured_root_value));
            if configured_root == current_root {
                return Some(crate::store::normalize_project_id(&project.id));
            }
        }
    }
    project_id_for_context(context)
}

fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }
    output.trim_matches('-').chars().take(80).collect()
}
