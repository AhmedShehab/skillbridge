use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::model::{FilePayload, ItemKind};

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_ITEM_BYTES: u64 = 25 * 1024 * 1024;

pub fn is_sensitive(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase().replace('\\', "/");
    let components = format!("/{lower}/");
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let extension = Path::new(&file)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    components.contains("/node_modules/")
        || components.contains("/.git/")
        || components.contains("/cache/")
        || components.contains("/caches/")
        || components.contains("/logs/")
        || components.contains("/sessions/")
        || components.contains("/history/")
        || file == ".env"
        || file.starts_with(".env.")
        || file.contains("credential")
        || file.contains("secret")
        || file.contains("token")
        || file.contains("auth")
        || matches!(
            extension,
            "db" | "sqlite" | "sqlite3" | "pem" | "key" | "p12" | "pfx"
        )
}

pub fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("unsafe relative path: {}", path.display());
    }
    Ok(())
}

pub fn collect_files(root: &Path, kind: ItemKind) -> Result<Vec<FilePayload>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if root.is_symlink() {
        anyhow::bail!("refusing to read symlinked item: {}", root.display());
    }

    if root.is_file() {
        if is_sensitive(root) {
            return Ok(Vec::new());
        }
        let metadata = fs::metadata(root)?;
        if metadata.len() > MAX_FILE_BYTES {
            anyhow::bail!("file exceeds 5 MiB safety limit: {}", root.display());
        }
        return Ok(vec![FilePayload {
            relative_path: root.file_name().map(PathBuf::from).unwrap_or_default(),
            bytes: fs::read(root).with_context(|| format!("reading {}", root.display()))?,
        }]);
    }

    let mut files = Vec::new();
    let mut total = 0_u64;
    let walker = WalkDir::new(root).follow_links(false);

    for entry in walker {
        let entry = entry.with_context(|| format!("walking {}", root.display()))?;
        let path = entry.path();
        if entry.file_type().is_symlink() {
            anyhow::bail!("refusing to follow symlink: {}", path.display());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if is_sensitive(path) {
            continue;
        }
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_FILE_BYTES {
            anyhow::bail!("file exceeds 5 MiB safety limit: {}", path.display());
        }
        total = total.saturating_add(metadata.len());
        if total > MAX_ITEM_BYTES {
            anyhow::bail!("item exceeds 25 MiB safety limit: {}", root.display());
        }
        let relative_path = if root.is_file() {
            root.file_name().map(PathBuf::from).unwrap_or_default()
        } else {
            path.strip_prefix(root)
                .with_context(|| {
                    format!("making {} relative to {}", path.display(), root.display())
                })?
                .to_path_buf()
        };
        validate_relative_path(&relative_path)?;
        if kind != ItemKind::Skill
            && relative_path
                .extension()
                .is_some_and(|extension| extension == "db")
        {
            continue;
        }
        files.push(FilePayload {
            relative_path,
            bytes: fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        });
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

pub fn hash_files(files: &[FilePayload]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(&file.bytes);
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

pub fn ensure_inside(root: &Path, candidate: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let parent = candidate.parent().unwrap_or(candidate);
    if parent.exists() {
        let parent = parent
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", parent.display()))?;
        if !parent.starts_with(&root) {
            anyhow::bail!(
                "refusing to write outside {}: {}",
                root.display(),
                candidate.display()
            );
        }
    } else {
        let mut current = parent.to_path_buf();
        while !current.exists() {
            if !current.pop() {
                anyhow::bail!(
                    "could not resolve safe destination: {}",
                    candidate.display()
                );
            }
        }
        let current = current.canonicalize()?;
        if !current.starts_with(&root) {
            anyhow::bail!(
                "refusing to write outside {}: {}",
                root.display(),
                candidate.display()
            );
        }
    }
    Ok(())
}

pub fn has_symlink_component(root: &Path, candidate: &Path) -> bool {
    if root.is_symlink() {
        return true;
    }
    let Ok(relative) = candidate.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if current.is_symlink() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names_are_rejected() {
        assert!(is_sensitive(Path::new(".env")));
        assert!(is_sensitive(Path::new("auth.json")));
        assert!(is_sensitive(Path::new("history/session.db")));
        assert!(!is_sensitive(Path::new("SKILL.md")));
    }

    #[test]
    fn relative_path_validation_rejects_escape() {
        assert!(validate_relative_path(Path::new("../escape")).is_err());
        assert!(validate_relative_path(Path::new("safe/file.md")).is_ok());
    }
}
