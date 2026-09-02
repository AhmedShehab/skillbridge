use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{CanonicalItem, FilePayload, ItemKind, Manifest, ProfileConfig, ScopeKind};
use crate::safety::{collect_files, hash_files, validate_relative_path};

const CONFIG_FILE: &str = "config.toml";
const MANIFEST_FILE: &str = "state/manifest.json";

pub fn normalize_project_id(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            result.push(character);
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-').to_string();
    if result.is_empty() {
        "current".to_string()
    } else {
        result.chars().take(80).collect()
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalStore {
    root: PathBuf,
}

impl CanonicalStore {
    pub fn init(root: PathBuf, home: &Path, project_id: Option<String>) -> Result<Self> {
        fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
        let store = Self { root };
        store.ensure_layout()?;
        let mut config = if store.config_path().exists() {
            store.config()?
        } else {
            ProfileConfig {
                schema_version: 1,
                profile_id: "default".to_string(),
                projects: Vec::new(),
            }
        };
        if let Some(project_id) = project_id {
            let root_path = std::env::current_dir().unwrap_or_else(|_| home.to_path_buf());
            let project_id = normalize_project_id(&project_id);
            if !config
                .projects
                .iter()
                .any(|project| project.id == project_id)
            {
                config.projects.push(crate::model::ProjectConfig {
                    id: project_id,
                    root: root_path.to_string_lossy().into_owned(),
                });
            }
        }
        store.save_config(&config)?;
        if !store.manifest_path().exists() {
            store.save_manifest(&Manifest::default())?;
        }
        Ok(store)
    }

    pub fn open(root: PathBuf) -> Result<Self> {
        if !root.exists() {
            anyhow::bail!("profile does not exist: {}; run init first", root.display());
        }
        let store = Self { root };
        store.ensure_layout()?;
        if !store.config_path().exists() {
            anyhow::bail!("profile is missing {CONFIG_FILE}");
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_FILE)
    }

    pub fn config(&self) -> Result<ProfileConfig> {
        let text = fs::read_to_string(self.config_path())?;
        toml::from_str(&text).context("parsing profile config.toml")
    }

    pub fn manifest(&self) -> Result<Manifest> {
        if !self.manifest_path().exists() {
            return Ok(Manifest::default());
        }
        let text = fs::read_to_string(self.manifest_path())?;
        serde_json::from_str(&text).context("parsing state/manifest.json")
    }

    pub fn save_config(&self, config: &ProfileConfig) -> Result<()> {
        let text = toml::to_string_pretty(config)?;
        atomic_write(&self.config_path(), text.as_bytes())
    }

    pub fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        let text = serde_json::to_string_pretty(manifest)?;
        atomic_write(&self.manifest_path(), text.as_bytes())
    }

    pub fn ensure_layout(&self) -> Result<()> {
        for path in [
            self.root.join("global/skills"),
            self.root.join("global/instructions"),
            self.root.join("global/memory"),
            self.root.join("projects"),
            self.root.join("state"),
        ] {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    pub fn item_root(&self, scope: ScopeKind, kind: ItemKind) -> PathBuf {
        let scope_root = match scope {
            ScopeKind::Global => self.root.join("global"),
            ScopeKind::Project => self.root.join("projects").join("current"),
        };
        scope_root.join(match kind {
            ItemKind::Skill => "skills",
            ItemKind::Instruction => "instructions",
            ItemKind::Memory => "memory",
        })
    }

    pub fn item_root_for_project(
        &self,
        scope: ScopeKind,
        kind: ItemKind,
        project_id: Option<&str>,
    ) -> PathBuf {
        if scope == ScopeKind::Global {
            return self.item_root(scope, kind);
        }
        self.root
            .join("projects")
            .join(normalize_project_id(project_id.unwrap_or("current")))
            .join(match kind {
                ItemKind::Skill => "skills",
                ItemKind::Instruction => "instructions",
                ItemKind::Memory => "memory",
            })
    }

    pub fn read_items(
        &self,
        scope: ScopeKind,
        project_id: Option<&str>,
    ) -> Result<Vec<CanonicalItem>> {
        let mut items = Vec::new();
        for kind in [ItemKind::Skill, ItemKind::Instruction, ItemKind::Memory] {
            let root = self.item_root_for_project(scope, kind, project_id);
            if !root.exists() {
                continue;
            }
            match kind {
                ItemKind::Skill => {
                    for entry in fs::read_dir(&root)? {
                        let entry = entry?;
                        let path = entry.path();
                        if !path.is_dir() || path.is_symlink() {
                            continue;
                        }
                        let skill_file = path.join("SKILL.md");
                        if !skill_file.exists() {
                            continue;
                        }
                        let files = collect_files(&path, kind)?;
                        let name = path
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("unnamed")
                            .to_string();
                        items.push(CanonicalItem {
                            id: format!("{kind}/{name}"),
                            kind,
                            scope,
                            relative_path: path.strip_prefix(&self.root)?.to_path_buf(),
                            content_hash: hash_files(&files),
                            files,
                        });
                    }
                }
                ItemKind::Instruction | ItemKind::Memory => {
                    for entry in fs::read_dir(&root)? {
                        let entry = entry?;
                        if !entry.file_type()?.is_file() {
                            continue;
                        }
                        let path = entry.path();
                        if crate::safety::is_sensitive(&path) {
                            continue;
                        }
                        let files = collect_files(&path, kind)?;
                        let name = path
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("unnamed.md")
                            .to_string();
                        items.push(CanonicalItem {
                            id: format!("{kind}/{name}"),
                            kind,
                            scope,
                            relative_path: path.strip_prefix(&self.root)?.to_path_buf(),
                            content_hash: hash_files(&files),
                            files,
                        });
                    }
                }
            }
        }
        items.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(items)
    }

    pub fn import_item(
        &self,
        scope: ScopeKind,
        project_id: Option<&str>,
        kind: ItemKind,
        name: &str,
        files: &[FilePayload],
    ) -> Result<bool> {
        validate_relative_path(Path::new(name))?;
        let destination = self
            .item_root_for_project(scope, kind, project_id)
            .join(name);
        let existing = collect_files(&destination, kind)?;
        if !existing.is_empty() && hash_files(&existing) == hash_files(files) {
            return Ok(false);
        }
        if !existing.is_empty() {
            anyhow::bail!(
                "canonical item already exists with different content: {}",
                destination.display()
            );
        }
        if kind == ItemKind::Skill {
            fs::create_dir_all(&destination)?;
            for file in files {
                validate_relative_path(&file.relative_path)?;
                let path = destination.join(&file.relative_path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                atomic_write(&path, &file.bytes)?;
            }
        } else {
            fs::create_dir_all(destination.parent().unwrap_or(&destination))?;
            let path = if destination.extension().is_some() {
                destination
            } else {
                destination.join(
                    files
                        .first()
                        .map(|file| file.relative_path.file_name().unwrap_or_default())
                        .unwrap_or_default(),
                )
            };
            if let Some(file) = files.first() {
                atomic_write(&path, &file.bytes)?;
            }
        }
        Ok(true)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("agent-sync-{}", std::process::id()));
    fs::write(&temp, bytes).with_context(|| format!("writing {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_profile_layout_and_round_trips_config() {
        let directory = tempdir().unwrap();
        let store =
            CanonicalStore::init(directory.path().join("profile"), directory.path(), None).unwrap();
        assert!(store.config_path().exists());
        assert!(store.manifest_path().exists());
        assert_eq!(store.config().unwrap().schema_version, 1);
    }
}
