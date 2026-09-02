use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    Global,
    Project,
}

impl fmt::Display for ScopeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::Project => write!(f, "project"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Skill,
    Instruction,
    Memory,
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skill => write!(f, "skill"),
            Self::Instruction => write!(f, "instruction"),
            Self::Memory => write!(f, "memory"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostContext {
    pub home: PathBuf,
    pub project_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FilePayload {
    pub relative_path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CanonicalItem {
    pub id: String,
    pub kind: ItemKind,
    pub scope: ScopeKind,
    pub relative_path: PathBuf,
    pub files: Vec<FilePayload>,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct DiscoveredItem {
    pub agent: String,
    pub kind: ItemKind,
    pub scope: ScopeKind,
    pub native_path: PathBuf,
    pub canonical_name: String,
    pub files: Vec<FilePayload>,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct Materialization {
    pub agent: String,
    pub kind: ItemKind,
    pub scope: ScopeKind,
    pub item_id: String,
    pub canonical_hash: String,
    pub target_root: PathBuf,
    pub files: Vec<FilePayload>,
    pub replace_existing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Create,
    Update,
    Delete,
    Conflict,
    Skip,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Conflict => "conflict",
            Self::Skip => "skip",
        };
        write!(f, "{label}")
    }
}

#[derive(Debug, Clone)]
pub struct PlannedChange {
    pub kind: ChangeKind,
    pub agent: String,
    pub scope: ScopeKind,
    pub item_id: String,
    pub target_root: PathBuf,
    pub canonical_hash: Option<String>,
    pub target_hash: Option<String>,
    pub materialization: Option<Materialization>,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct SyncPlan {
    pub changes: Vec<PlannedChange>,
}

impl SyncPlan {
    pub fn has_conflicts(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.kind == ChangeKind::Conflict)
    }

    pub fn is_empty(&self) -> bool {
        self.changes
            .iter()
            .all(|change| matches!(change.kind, ChangeKind::Skip))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub schema_version: u32,
    pub profile_id: String,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub id: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<ManifestEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub agent: String,
    pub kind: ItemKind,
    pub scope: ScopeKind,
    pub item_id: String,
    pub target_root: String,
    pub canonical_hash: String,
    pub target_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.level == DiagnosticLevel::Error
    }
}
