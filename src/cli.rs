use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::model::{HostContext, ScopeKind};

#[derive(Debug, Parser)]
#[command(
    name = "agent-sync",
    version,
    about = "Sync AI coding agent skills and instructions"
)]
pub struct Cli {
    /// Override the home directory used for adapter discovery. Useful for tests.
    #[arg(long, global = true, env = "AGENT_SYNC_HOME")]
    pub home: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new canonical profile.
    Init {
        /// Directory that will contain the canonical profile.
        #[arg(long)]
        profile: PathBuf,
        /// Optional stable ID for the current project.
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Discover native files without changing anything.
    Scan {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value_t = ScopeArg::Both)]
        scope: ScopeArg,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Import discovered native files into the canonical profile.
    Import {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value_t = ScopeArg::Both)]
        scope: ScopeArg,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Show the changes that would be materialized into agent directories.
    Plan {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value_t = ScopeArg::Both)]
        scope: ScopeArg,
        #[arg(long)]
        project: Option<PathBuf>,
        /// Resolution used for conflicts. `canonical` wins; `target` imports the target first.
        #[arg(long, value_enum)]
        resolve: Option<ResolveArg>,
    },
    /// Apply a reviewed plan. Requires --yes to make writes explicit.
    Apply {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value_t = ScopeArg::Both)]
        scope: ScopeArg,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
        #[arg(long, value_enum)]
        resolve: Option<ResolveArg>,
    },
    /// Report drift between the canonical profile and target agents.
    Status {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, value_enum, default_value_t = ScopeArg::Both)]
        scope: ScopeArg,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Check the profile, paths, permissions, and supported agents.
    Doctor {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List built-in adapters and their capabilities.
    Adapters,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ScopeArg {
    Global,
    Project,
    Both,
}

impl From<ScopeArg> for Option<ScopeKind> {
    fn from(value: ScopeArg) -> Self {
        match value {
            ScopeArg::Global => Some(ScopeKind::Global),
            ScopeArg::Project => Some(ScopeKind::Project),
            ScopeArg::Both => None,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ResolveArg {
    Canonical,
    Target,
}

impl Cli {
    pub fn home_dir(&self) -> Result<PathBuf> {
        self.home
            .clone()
            .or_else(dirs::home_dir)
            .context("could not determine the home directory; pass --home or AGENT_SYNC_HOME")
    }

    pub fn context(home: &Path, project: Option<PathBuf>) -> HostContext {
        let project_root = project
            .or_else(|| std::env::current_dir().ok())
            .map(|path| path.canonicalize().unwrap_or(path));
        HostContext {
            home: home.to_path_buf(),
            project_root,
        }
    }
}
