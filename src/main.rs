mod adapters;
mod cli;
mod model;
mod safety;
mod store;
mod sync_engine;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};
use crate::model::HostContext;

fn select_adapters<'a>(
    registry: &'a adapters::AdapterRegistry,
    requested: Option<&str>,
    context: &HostContext,
) -> Result<Vec<&'a dyn adapters::AgentAdapter>> {
    let selected = registry.select(requested);
    if let Some(requested) = requested {
        if selected.is_empty() {
            anyhow::bail!("unknown adapter `{requested}`; run `skillbridge adapters`");
        }
        return Ok(selected);
    }
    Ok(selected
        .into_iter()
        .filter(|adapter| adapter.detect(context))
        .collect())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let registry = adapters::registry();
    let home = cli.home_dir()?;

    match cli.command {
        Command::Init {
            profile,
            project_id,
        } => {
            let store = store::CanonicalStore::init(profile, &home, project_id)?;
            println!(
                "Initialized SkillBridge profile at {}",
                store.root().display()
            );
            println!("Add this directory to Git to sync it across machines.");
        }
        Command::Scan {
            agent,
            scope,
            project,
        } => {
            let context = Cli::context(&home, project);
            let adapters = select_adapters(&registry, agent.as_deref(), &context)?;
            let items = sync_engine::scan(&adapters, &context, scope)?;
            if items.is_empty() {
                println!("No managed skills or instruction files found.");
            } else {
                for item in items {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        item.agent,
                        item.scope,
                        item.kind,
                        item.native_path.display(),
                        item.content_hash
                    );
                }
            }
        }
        Command::Import {
            profile,
            agent,
            scope,
            project,
        } => {
            let store = store::CanonicalStore::open(profile)?;
            let context = Cli::context(&home, project);
            let adapters = select_adapters(&registry, agent.as_deref(), &context)?;
            let result = sync_engine::import(&store, &adapters, &context, scope)?;
            println!(
                "Imported {} item(s), skipped {}, conflict(s) {}.",
                result.imported, result.skipped, result.conflicts
            );
            if result.conflicts > 0 {
                anyhow::bail!("import stopped because conflicting canonical files were found");
            }
        }
        Command::Plan {
            profile,
            agent,
            scope,
            project,
            resolve,
        } => {
            let store = store::CanonicalStore::open(profile)?;
            let context = Cli::context(&home, project);
            let adapters = select_adapters(&registry, agent.as_deref(), &context)?;
            let plan = sync_engine::plan(&store, &adapters, &context, scope)?;
            sync_engine::print_plan(&plan);
            if plan.has_conflicts() && resolve.is_none() {
                anyhow::bail!("plan contains conflicts; resolve them before applying");
            }
        }
        Command::Apply {
            profile,
            agent,
            scope,
            project,
            yes,
            resolve,
        } => {
            let store = store::CanonicalStore::open(profile)?;
            let context = Cli::context(&home, project);
            let adapters = select_adapters(&registry, agent.as_deref(), &context)?;
            let plan = sync_engine::plan(&store, &adapters, &context, scope)?;
            sync_engine::print_plan(&plan);
            if plan.has_conflicts() && resolve.is_none() {
                anyhow::bail!("apply refused because the plan contains conflicts");
            }
            if !yes && !plan.is_empty() {
                anyhow::bail!("apply requires --yes after reviewing the plan");
            }
            let project_id = sync_engine::project_id_for_store(&store, &context);
            let result = sync_engine::apply(&store, &plan, resolve, project_id.as_deref())?;
            println!(
                "Applied {} file operation(s); skipped {}.",
                result.applied, result.skipped
            );
        }
        Command::Status {
            profile,
            agent,
            scope,
            project,
        } => {
            let store = store::CanonicalStore::open(profile)?;
            let context = Cli::context(&home, project);
            let adapters = select_adapters(&registry, agent.as_deref(), &context)?;
            let plan = sync_engine::plan(&store, &adapters, &context, scope)?;
            sync_engine::print_status(&plan);
            if plan.has_conflicts() {
                std::process::exit(2);
            }
        }
        Command::Doctor { profile, project } => {
            let store = store::CanonicalStore::open(profile)?;
            let context = Cli::context(&home, project);
            let diagnostics = sync_engine::doctor(&store, &registry, &context)?;
            let mut errors = 0;
            for diagnostic in diagnostics {
                println!("{}\t{}", diagnostic.level, diagnostic.message);
                if diagnostic.is_error() {
                    errors += 1;
                }
            }
            if errors > 0 {
                anyhow::bail!("doctor found {errors} error(s)");
            }
        }
        Command::Adapters => {
            for adapter in registry.all() {
                println!(
                    "{}\t{}\t{}\t{}",
                    adapter.id(),
                    adapter.name(),
                    adapter.summary(),
                    adapter.capabilities()
                );
            }
        }
    }
    Ok(())
}
