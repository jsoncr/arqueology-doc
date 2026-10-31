//! Despacho de comandos y resolución de proyecto.

use anyhow::{bail, Context, Result};
use archeology_core::{t, Project};
use std::path::PathBuf;

pub mod agent;
pub mod ask;
pub mod check;
pub mod config;
pub mod context_cmd;
pub mod context_view;
pub mod doctor;
pub mod graph;
pub mod init;
pub mod inspect;
pub mod install_location;
pub mod plan;
pub mod projects;
pub mod projects_delete;
pub mod report;
pub mod resume;
pub mod review;
pub mod scan;
pub mod site;
// `adoc spec` está desactivado temporalmente mientras se rediseña: `spec.rs`
// sigue en el repositorio pero no se compila. La detección de OpenSpec y el
// cumplimiento (status, review, check, plan) usan `specs`.
// pub mod spec;
pub mod specs;
pub mod status;
pub mod status_view;
pub mod sync;
pub mod task;
pub mod toolchain;
pub mod uninstall;
pub mod upgrade;

/// Resuelve el proyecto objetivo: `--project <nombre|id>` contra el registro,
/// o descubrimiento ascendente desde el directorio actual (RF-173).
pub fn resolve_project(project_arg: &Option<String>) -> Result<Project> {
    if let Some(name_or_id) = project_arg {
        let registry = archeology_registry::Registry::load()?;
        let entry = registry
            .resolve(name_or_id)?
            .with_context(|| t!("cli.commands.project_not_in_registry", name = name_or_id))?;
        return Ok(Project::open(&PathBuf::from(&entry.path))?);
    }
    Ok(Project::discover(&std::env::current_dir()?)?)
}

/// Registra actividad del proyecto en el registro global (tolerante a fallos).
pub fn touch_registry(project: &Project, command: &str) {
    let result = archeology_registry::Registry::update(|reg| {
        reg.upsert(
            &project.config.project.id,
            &project.config.project.name,
            &project.root,
            command,
        )
    });
    if let Err(e) = result {
        tracing::warn!("{}", t!("cli.commands.registry_update_failed", error = e));
    }
}

pub fn run(command: super::Commands, project_arg: Option<String>) -> Result<()> {
    use super::Commands::*;
    match command {
        Init(args) => init::run(args),
        Config { cmd } => config::run(cmd),
        Scan(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "scan");
            let _lock = p.lock("scan")?;
            scan::run(&p, args)
        }
        Plan(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "plan");
            let _lock = p.lock("plan")?;
            plan::run(&p, args)
        }
        Inspect(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "inspect");
            let _lock = p.lock("inspect")?;
            inspect::run(&p, args)
        }
        Task { cmd } => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "task");
            // Solo los subcomandos que cambian el estado; `task list` y
            // `task show` siguen disponibles durante un `inspect`.
            let _lock = cmd.mutates().then(|| p.lock("task")).transpose()?;
            task::run(&p, cmd)
        }
        Review(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "review");
            review::run(&p, args)
        }
        Check(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "check");
            let code = check::run(&p, args)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Sync(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "sync");
            let _lock = p.lock("sync")?;
            sync::run(&p, args)
        }
        Site {
            cmd: site::SiteCmd::Setup { version },
        } => site::setup(&version),
        Site { cmd } => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "site");
            site::run(&p, cmd)
        }
        Agent { cmd } => agent::run(cmd, project_arg),
        Projects { cmd } => projects::run(cmd),
        Resume(args) => resume::run(args, project_arg),
        Preview(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "preview");
            site::preview(&p, args.port, args.no_open, args.yes)
        }
        Report(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "report");
            report::run(&p, args)
        }
        Graph(args) => {
            let p = resolve_project(&project_arg)?;
            graph::run(&p, args)
        }
        Context(args) => {
            // Solo lectura: sin lock ni registro. Sin proyecto o con un error
            // de uso, código 1 (lo devuelve `main` al propagar el error).
            let p = resolve_project(&project_arg)?;
            let code = context_cmd::run(&p, args)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Ask(args) => {
            let p = resolve_project(&project_arg)?;
            touch_registry(&p, "ask");
            // Como `inspect`: el agente escribe en el vault y en el espacio de trabajo.
            let _lock = p.lock("ask")?;
            ask::run(&p, args)
        }
        Status(args) => {
            let p = match resolve_project(&project_arg) {
                Ok(p) => p,
                // Sin proyecto en modo texto: se explica cómo empezar.
                Err(e)
                    if args.format == "text"
                        && project_arg.is_none()
                        && matches!(
                            e.downcast_ref::<archeology_core::Error>(),
                            Some(archeology_core::Error::ProjectNotFound)
                        ) =>
                {
                    println!("{}", t!("cli.status.not_initialized"));
                    println!("\n{} → adoc init", t!("cli.status.section.next"));
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            status::run(&p, args)
        }
        Uninstall(args) => uninstall::run(args),
        Upgrade(args) => upgrade::run(args),
        Doctor(args) => {
            let code = doctor::run(args, project_arg)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
    }
}

/// Confirma con el usuario salvo `--yes` o entrada no interactiva.
pub fn confirm(prompt: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !atty_is_interactive() {
        bail!(t!("cli.commands.confirm_required"));
    }
    eprint!("{prompt} {} ", t!("cli.commands.confirm_suffix"));
    use std::io::Write;
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    ))
}

fn atty_is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Pregunta de texto simple en modo interactivo.
pub fn ask(prompt: &str, default: &str) -> Result<String> {
    if !atty_is_interactive() {
        return Ok(default.to_string());
    }
    if default.is_empty() {
        eprint!("{prompt}: ");
    } else {
        eprint!("{prompt} [{default}]: ");
    }
    use std::io::Write;
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let v = line.trim();
    Ok(if v.is_empty() {
        default.to_string()
    } else {
        v.to_string()
    })
}
