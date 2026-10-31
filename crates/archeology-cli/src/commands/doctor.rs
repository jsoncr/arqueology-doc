//! `doctor` — verifica los requisitos del CLI y del proyecto.
//!
//! Requeridos (fallo → código de salida 1): directorio de configuración
//! escribible, registro legible y, dentro de un proyecto, configuración válida
//! y adaptador por defecto disponible. El resto son avisos: habilitan
//! funciones concretas (historial git, sitio, validación de OpenSpec).

use super::toolchain;
use anyhow::Result;
use archeology_core::config::AdapterMode;
use archeology_core::{t, Project};
use clap::Args;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Args)]
pub struct DoctorArgs {
    /// Formato de salida.
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Serialize)]
struct Check {
    name: String,
    level: Level,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

struct Report(Vec<Check>);

impl Report {
    fn add(&mut self, name: &str, level: Level, detail: impl Into<String>, hint: Option<String>) {
        self.0.push(Check {
            name: name.into(),
            level,
            detail: detail.into(),
            hint,
        });
    }
}

pub fn run(args: DoctorArgs, project_arg: Option<String>) -> Result<i32> {
    let pb = crate::ui::spinner(t!("cli.doctor.checking_config"));
    let mut r = Report(Vec::new());
    r.add(
        "adoc",
        Level::Ok,
        t!("cli.doctor.version", version = env!("CARGO_PKG_VERSION")),
        None,
    );

    // Versión del contrato JSON de `adoc context` que entiende este binario.
    r.add(
        "context",
        Level::Ok,
        t!(
            "cli.doctor.context_contract",
            version = archeology_core::zone::SCHEMA_VERSION
        ),
        None,
    );
    check_config_dir(&mut r);
    check_registry(&mut r);

    // Proyecto actual o `--project`: opcional para doctor.
    let project = match &project_arg {
        Some(_) => match super::resolve_project(&project_arg) {
            Ok(p) => Some(p),
            Err(e) => {
                r.add("proyecto", Level::Fail, e.to_string(), None);
                None
            }
        },
        None => match Project::discover(&std::env::current_dir()?) {
            Ok(p) => Some(p),
            Err(archeology_core::Error::ProjectNotFound) => None,
            Err(e) => {
                r.add(
                    "proyecto",
                    Level::Fail,
                    t!("cli.doctor.invalid_config", error = e),
                    Some(t!("cli.doctor.invalid_config_hint")),
                );
                None
            }
        },
    };

    pb.set_message(t!("cli.doctor.checking_tools"));
    check_git(&mut r);
    let node_ok = check_node(&mut r);
    check_vitepress(&mut r, project.as_ref(), node_ok);
    check_openspec(&mut r, project.as_ref());
    pb.set_message(t!("cli.doctor.checking_agents"));
    check_agents(&mut r, project.as_ref());
    if let Some(p) = &project {
        check_project(&mut r, p);
    } else if project_arg.is_none() && !r.0.iter().any(|c| c.name == "proyecto") {
        r.add("proyecto", Level::Ok, t!("cli.doctor.no_project"), None);
    }

    pb.set_message(t!("cli.upgrade.checking"));
    check_update(&mut r);

    pb.finish_and_clear();
    let failed = r.0.iter().any(|c| c.level == Level::Fail);
    if args.format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": !failed,
                "context_schema_version": archeology_core::zone::SCHEMA_VERSION,
                "checks": r.0,
            }))?
        );
    } else {
        for c in &r.0 {
            let mark = match c.level {
                Level::Ok => "✓",
                Level::Warn => "!",
                Level::Fail => "✗",
            };
            println!("{mark} {:<14} {}", c.name, c.detail);
            if let Some(h) = &c.hint {
                println!("  {:<14} → {h}", "");
            }
        }
        let warns = r.0.iter().filter(|c| c.level == Level::Warn).count();
        let fails = r.0.iter().filter(|c| c.level == Level::Fail).count();
        println!(
            "\n{}",
            t!("cli.doctor.summary", fails = fails, warns = warns)
        );
    }
    Ok(if failed { 1 } else { 0 })
}

/// Aviso de versión nueva (caché de 24 h; sin red o con
/// `ADOC_NO_UPDATE_CHECK=1` no aparece). Nunca es un fallo.
fn check_update(r: &mut Report) {
    let Some(c) = super::upgrade::passive_check() else {
        return;
    };
    if c.update_available {
        r.add(
            "actualización",
            Level::Warn,
            t!(
                "cli.upgrade.notice_available",
                latest = c.latest,
                current = super::upgrade::CURRENT_VERSION
            ),
            Some("adoc upgrade".into()),
        );
    } else {
        r.add(
            "actualización",
            Level::Ok,
            t!("cli.upgrade.notice_up_to_date", latest = c.latest),
            None,
        );
    }
}

fn check_config_dir(r: &mut Report) {
    match archeology_registry::config_dir() {
        Ok(dir) => {
            let probe = dir.join(format!(".doctor-{}", std::process::id()));
            let writable = fs::create_dir_all(&dir)
                .and_then(|_| fs::write(&probe, b""))
                .and_then(|_| fs::remove_file(&probe));
            match writable {
                Ok(()) => r.add("configuración", Level::Ok, dir.display().to_string(), None),
                Err(e) => r.add(
                    "configuración",
                    Level::Fail,
                    t!(
                        "cli.doctor.config_not_writable",
                        path = dir.display(),
                        error = e
                    ),
                    Some(t!("cli.doctor.config_not_writable_hint")),
                ),
            }
        }
        Err(e) => r.add(
            "configuración",
            Level::Fail,
            e.to_string(),
            Some(t!("cli.doctor.config_dir_hint")),
        ),
    }
}

fn check_registry(r: &mut Report) {
    match archeology_registry::Registry::load() {
        Ok(reg) => {
            let missing = reg
                .projects
                .iter()
                .filter(|p| !PathBuf::from(&p.path).is_dir())
                .count();
            if missing > 0 {
                r.add(
                    "registro",
                    Level::Warn,
                    t!(
                        "cli.doctor.registry_missing_paths",
                        total = reg.projects.len(),
                        missing = missing
                    ),
                    Some("adoc projects prune".into()),
                );
            } else {
                r.add(
                    "registro",
                    Level::Ok,
                    t!("cli.doctor.registry_ok", total = reg.projects.len()),
                    None,
                );
            }
        }
        Err(e) => r.add(
            "registro",
            Level::Fail,
            t!("cli.doctor.registry_unreadable", error = format!("{e:#}")),
            Some(t!("cli.doctor.registry_unreadable_hint")),
        ),
    }
}

fn check_git(r: &mut Report) {
    match toolchain::tool_version("git") {
        Some(v) => r.add("git", Level::Ok, v, None),
        None => r.add(
            "git",
            Level::Warn,
            t!("cli.doctor.git_missing"),
            Some(t!("cli.doctor.git_missing_hint")),
        ),
    }
}

fn check_node(r: &mut Report) -> bool {
    match toolchain::node_version() {
        Some(v) if toolchain::major(&v).unwrap_or(0) >= toolchain::MIN_NODE_MAJOR => {
            let npm = toolchain::npm_version()
                .map(|n| format!(" · npm {n}"))
                .unwrap_or_else(|| t!("cli.doctor.npm_missing"));
            r.add("node", Level::Ok, format!("{v}{npm}"), None);
            true
        }
        Some(v) => {
            r.add(
                "node",
                Level::Warn,
                t!(
                    "cli.doctor.node_old",
                    version = v,
                    min = toolchain::MIN_NODE_MAJOR
                ),
                Some(t!("cli.doctor.node_old_hint")),
            );
            false
        }
        None => {
            r.add(
                "node",
                Level::Warn,
                t!("cli.doctor.node_missing"),
                Some(t!(
                    "cli.doctor.install_node_hint",
                    min = toolchain::MIN_NODE_MAJOR
                )),
            );
            false
        }
    }
}

fn check_vitepress(r: &mut Report, project: Option<&Project>, node_ok: bool) {
    match toolchain::find_vitepress(project) {
        Some(vp) => {
            let detail = format!(
                "{} ({}) — {}",
                vp.version.as_deref().unwrap_or("?"),
                vp.source.label(),
                vp.package_dir.display()
            );
            let global_with_import = vp.source == toolchain::VitepressSource::Global
                && project
                    .map(toolchain::config_imports_vitepress)
                    .unwrap_or(false);
            if global_with_import {
                r.add(
                    "vitepress",
                    Level::Warn,
                    detail,
                    Some(t!("cli.doctor.vitepress_global_import_hint")),
                );
            } else {
                r.add("vitepress", Level::Ok, detail, None);
            }
        }
        None => r.add(
            "vitepress",
            Level::Warn,
            t!("cli.doctor.vitepress_missing"),
            Some(if node_ok {
                t!("cli.doctor.vitepress_missing_hint")
            } else {
                t!(
                    "cli.doctor.vitepress_missing_no_node_hint",
                    min = toolchain::MIN_NODE_MAJOR
                )
            }),
        ),
    }
}

fn check_openspec(r: &mut Report, project: Option<&Project>) {
    let uses_openspec = project
        .map(|p| p.config.openspec_enabled(&p.root))
        .unwrap_or(false);
    match toolchain::tool_version("openspec") {
        Some(v) => r.add("openspec", Level::Ok, v, None),
        None if uses_openspec => r.add(
            "openspec",
            Level::Warn,
            t!("cli.doctor.openspec_missing"),
            Some("npm i -g @fission-ai/openspec".into()),
        ),
        None => r.add(
            "openspec",
            Level::Ok,
            t!("cli.doctor.openspec_optional"),
            None,
        ),
    }
}

fn check_agents(r: &mut Report, project: Option<&Project>) {
    let config = project.map(|p| p.config.clone()).unwrap_or_default();
    let default = &config.agent.default;
    for (name, adapter) in &config.agent.adapters {
        let st = archeology_agents::check_adapter(name, adapter);
        let is_default = name == default;
        let label = if is_default {
            format!("agente:{name}*")
        } else {
            format!("agente:{name}")
        };
        let level = match (st.available, is_default && project.is_some()) {
            (true, _) => Level::Ok,
            (false, true) => Level::Fail,
            (false, false) => Level::Warn,
        };
        let hint = (!st.available).then(|| {
            if !adapter.trusted {
                t!("cli.doctor.agent_untrusted_hint")
            } else if is_default {
                t!("cli.doctor.agent_default_unavailable_hint")
            } else {
                t!("cli.doctor.agent_optional_hint")
            }
        });
        let mode = match adapter.mode {
            AdapterMode::Headless => "headless",
            AdapterMode::Export => "export",
        };
        r.add(&label, level, format!("{mode} · {}", st.detail), hint);
        for w in archeology_agents::adapter_warnings(name, adapter) {
            r.add(
                &label,
                if is_default && project.is_some() {
                    Level::Fail
                } else {
                    Level::Warn
                },
                w,
                None,
            );
        }
    }
    if !config.agent.adapters.contains_key(default) {
        r.add(
            "agente",
            Level::Fail,
            t!("cli.doctor.default_not_configured", agent = default),
            Some(t!("cli.doctor.default_not_configured_hint")),
        );
    }
}

fn check_project(r: &mut Report, p: &Project) {
    r.add(
        "proyecto",
        Level::Ok,
        format!("{} ({})", p.config.project.name, p.root.display()),
        None,
    );
    r.add(
        "espacio",
        Level::Ok,
        p.state_dir().display().to_string(),
        None,
    );
    if let Err(e) = archeology_core::state::State::load(&p.state_file()) {
        r.add(
            "estado",
            Level::Fail,
            t!("cli.doctor.state_unreadable", error = e),
            Some(t!("cli.doctor.state_unreadable_hint")),
        );
    }
    if !p.inventory_file().is_file() {
        r.add(
            "inventario",
            Level::Warn,
            t!("cli.doctor.no_inventory"),
            Some("adoc scan".into()),
        );
    }
    if p.config.site.enabled {
        let vp = p.docs_dir().join(".vitepress");
        if !vp.join("config.mts").is_file() && !vp.join("config.ts").is_file() {
            r.add(
                "sitio",
                Level::Warn,
                t!("cli.doctor.site_config_missing"),
                Some(t!("cli.doctor.site_config_missing_hint")),
            );
        }
    }
    // El registro debe apuntar a la ruta actual (RF-175).
    if let Ok(reg) = archeology_registry::Registry::load() {
        match reg.projects.iter().find(|e| e.id == p.config.project.id) {
            None => r.add(
                "registro",
                Level::Warn,
                t!("cli.doctor.not_registered"),
                Some("adoc projects add .".into()),
            ),
            Some(e)
                if std::path::Path::new(&e.path)
                    != archeology_core::paths::canonicalize(&p.root).unwrap_or(p.root.clone()) =>
            {
                r.add(
                    "registro",
                    Level::Warn,
                    t!("cli.doctor.registered_elsewhere", path = e.path),
                    Some(t!("cli.doctor.registered_elsewhere_hint")),
                )
            }
            _ => {}
        }
    }
}
