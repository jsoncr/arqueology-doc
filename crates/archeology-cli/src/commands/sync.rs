//! `sync` — detecta cambios en fuentes, marca notas stale y reabre las
//! tareas afectadas (§8.11).
//!
//! Dos vistas de los cambios:
//!
//! - **Fuentes documentadas**: archivos citados en `sources:` de las notas.
//!   Se comparan con los hashes guardados (o con `git diff <ref>` si se usa
//!   `--since`). Son los que vuelven `stale` a las notas.
//! - **Cambios fuera de la documentación**: archivos que ninguna nota cita y
//!   que cambiaron desde el último `adoc scan`. El inventario no guarda
//!   hashes por archivo, así que se usa lo más barato y fiable: los
//!   candidatos de git (`git diff --name-status <commit del scan>` sobre el
//!   árbol de trabajo + archivos sin seguimiento no ignorados) y, para los
//!   modificados o añadidos, solo los que tienen mtime posterior al scan
//!   (descarta cambios sin commitear que ya existían al escanear). Sin git
//!   o sin inventario no se calculan. Con `--since`, son los archivos del
//!   diff que ninguna nota cita.

use anyhow::Result;
use archeology_core::index::{self, MarkResult};
use archeology_core::planner;
use archeology_core::state::State;
use archeology_core::{t, Project};
use clap::Args;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::ui;

#[derive(Args)]
pub struct SyncArgs {
    /// Comparar contra una ref git en vez de los hashes guardados.
    #[arg(long)]
    since: Option<String>,
    /// Mostrar qué cambiaría sin modificar notas, tareas ni estado.
    #[arg(long)]
    dry_run: bool,
    /// Formato de salida.
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,
}

/// Resultado de `sync` (contrato JSON estable).
#[derive(Serialize)]
struct SyncReport {
    compared_against: ComparedAgainst,
    dry_run: bool,
    watched: Watched,
    changed: Vec<ChangedFile>,
    /// Cambios en archivos que ninguna nota cita.
    uncovered: Vec<UncoveredFile>,
    /// `git` (calculado) o `unavailable` (sin git o sin inventario).
    uncovered_method: &'static str,
    stale_notes: Vec<String>,
    /// Notas afectadas con `locked: true` (no se tocan).
    locked_notes: Vec<String>,
    reopened_tasks: Vec<String>,
    next_step: Option<NextStep>,
}

#[derive(Serialize)]
struct ComparedAgainst {
    /// `hashes` (guardados en el último sync/scan) o `git` (`--since`).
    mode: &'static str,
    /// Ref de git con `--since`.
    reference: Option<String>,
    /// Fecha del inventario (base de los cambios fuera de la documentación).
    inventory_at: Option<String>,
}

#[derive(Serialize)]
struct Watched {
    /// Archivos citados por las notas.
    files: usize,
    /// Notas que citan al menos un archivo.
    notes: usize,
}

#[derive(Serialize)]
struct ChangedFile {
    path: String,
    /// `modified`, `added` o `deleted`.
    change: &'static str,
    /// Notas que lo citan (rutas relativas a la raíz).
    notes: Vec<String>,
}

#[derive(Serialize)]
struct UncoveredFile {
    path: String,
    change: &'static str,
}

#[derive(Serialize)]
struct NextStep {
    command: String,
    reason: String,
}

/// Elementos que se listan por sección en texto.
const MAX_LIST: usize = 10;
/// Notas por archivo cambiado en texto.
const MAX_NOTES_PER_FILE: usize = 3;
/// Ancho de la columna de etiquetas.
const LABEL_WIDTH: usize = 20;

pub fn run(project: &Project, args: SyncArgs) -> Result<()> {
    let pb = crate::ui::spinner(t!("cli.sync.comparing"));
    let mut state = State::load(&project.state_file())?;
    // El índice se reconstruye desde las notas (fuente de verdad).
    state.source_to_notes = index::source_index(project);

    // Hashes actuales de todas las fuentes citadas.
    let mut current: BTreeMap<String, String> = BTreeMap::new();
    for rel in state.source_to_notes.keys() {
        if let Some(h) = project.source_hash(rel) {
            current.insert(rel.clone(), h);
        }
    }

    // Cambios con su tipo (M/A/D).
    let since_diff: Option<Vec<(&'static str, String)>> = match &args.since {
        Some(reference) => match diff_name_status(&project.root, reference) {
            Some(files) => Some(files),
            None => anyhow::bail!(t!("cli.sync.git_diff_failed", reference = reference)),
        },
        None => None,
    };
    let changed: Vec<(&'static str, String)> = match &since_diff {
        Some(files) => files.clone(),
        None => {
            let mut changed = Vec::new();
            for rel in state.source_to_notes.keys() {
                // Sin hash previo = primera vez que se cita: es la línea base.
                let Some(old) = state.file_hashes.get(rel) else {
                    continue;
                };
                match current.get(rel) {
                    Some(h) if h == old => {}
                    Some(_) => changed.push(("modified", rel.clone())),
                    None => changed.push(("deleted", rel.clone())),
                }
            }
            changed
        }
    };
    let changed_paths: Vec<String> = changed.iter().map(|(_, p)| p.clone()).collect();

    // Fuera de la documentación: con `--since`, el resto del diff; si no,
    // lo que cambió desde el último scan según git.
    let inventory: Option<archeology_core::model::Inventory> =
        fs::read_to_string(project.inventory_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
    let watched_set: BTreeSet<String> = state
        .source_to_notes
        .keys()
        .map(|k| index::normalize_rel_path(k))
        .collect();
    let (uncovered_raw, uncovered_method) = match (&since_diff, &inventory) {
        (Some(files), _) => (Some(files.clone()), "git"),
        (None, Some(inv)) => match changed_since_scan(project, inv) {
            Some(files) => (Some(files), "git"),
            None => (None, "unavailable"),
        },
        (None, None) => (None, "unavailable"),
    };
    let uncovered: Vec<UncoveredFile> = uncovered_raw
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, p)| !watched_set.contains(&index::normalize_rel_path(p)))
        .filter(|(_, p)| !is_generated(project, p))
        .map(|(change, path)| UncoveredFile { path, change })
        .collect();

    pb.finish_and_clear();
    let affected = state.affected_notes(&changed_paths);

    // Marcar notas stale respetando `locked` (RF-121, RF-123).
    let mut marked = Vec::new();
    let mut locked = Vec::new();
    for note in &affected {
        let path = project.root.join(note);
        let result = if args.dry_run {
            MarkResult::Marked
        } else {
            index::mark_stale(&path)?
        };
        match result {
            MarkResult::Marked => marked.push(note.clone()),
            MarkResult::Locked => locked.push(note.clone()),
            MarkResult::AlreadyStale | MarkResult::NoFrontmatter => {}
        }
        // Motivo (fuentes cambiadas) para la sección de re-verificación de la tarea.
        if !args.dry_run && matches!(result, MarkResult::Marked | MarkResult::AlreadyStale) {
            for path in &changed_paths {
                if state
                    .affected_notes(std::slice::from_ref(path))
                    .contains(note)
                {
                    state.add_stale_reason(note, path);
                }
            }
        }
    }

    // Reabrir las tareas que producen notas afectadas (RF-122).
    let reopened = reopen_tasks(project, &mut state, &marked, args.dry_run)?;

    let watched_notes: BTreeSet<&String> = state.source_to_notes.values().flatten().collect();
    let report = SyncReport {
        compared_against: ComparedAgainst {
            mode: if args.since.is_some() {
                "git"
            } else {
                "hashes"
            },
            reference: args.since.clone(),
            inventory_at: inventory.as_ref().map(|i| i.generated_at.clone()),
        },
        dry_run: args.dry_run,
        watched: Watched {
            files: state.source_to_notes.len(),
            notes: watched_notes.len(),
        },
        changed: changed
            .iter()
            .filter(|(_, p)| {
                // Con `--since`, solo los citados; el resto va a `uncovered`.
                args.since.is_none() || watched_set.contains(&index::normalize_rel_path(p))
            })
            .map(|(change, path)| ChangedFile {
                path: path.clone(),
                change,
                notes: state.affected_notes(std::slice::from_ref(path)),
            })
            .collect(),
        uncovered,
        uncovered_method,
        stale_notes: marked,
        locked_notes: locked,
        reopened_tasks: reopened,
        next_step: None,
    };
    let report = SyncReport {
        next_step: next_step(&report, inventory.is_some()),
        ..report
    };

    if args.format == "json" {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render(project, &report));
    }

    if args.dry_run {
        return Ok(());
    }

    // Nueva línea base: un cambio se notifica una sola vez.
    for (rel, h) in &current {
        state.file_hashes.insert(rel.clone(), h.clone());
    }
    for c in &changed_paths {
        if !current.contains_key(c) {
            state.file_hashes.remove(c);
        }
    }
    state.set_stale_notes(index::stale_notes(project));
    // Commit documentado: `sync` acaba de comparar las fuentes con los hashes
    // (lo usa `adoc context` para avisar de cambios posteriores). `scan` no
    // lo escribe porque no re-verifica las notas.
    state.knowledge_commit = archeology_core::gitcmd::head_commit(&project.root);
    state.save(&project.state_file())?;
    index::write_mocs(project)?;
    Ok(())
}

/// Siguiente paso sugerido (None: nada que hacer).
fn next_step(r: &SyncReport, has_inventory: bool) -> Option<NextStep> {
    let step = |command: &str, reason: String| {
        Some(NextStep {
            command: command.to_string(),
            reason,
        })
    };
    if !has_inventory {
        return step("adoc scan", t!("cli.sync.next.scan"));
    }
    if !r.uncovered.is_empty() {
        return step(
            "adoc scan && adoc plan",
            t!("cli.sync.next.scan_plan", count = r.uncovered.len()),
        );
    }
    if !r.reopened_tasks.is_empty() || !r.stale_notes.is_empty() {
        return step("adoc inspect", t!("cli.sync.next.inspect"));
    }
    if r.watched.files == 0 {
        return step("adoc inspect", t!("cli.sync.next.no_watched"));
    }
    None
}

/// `git diff --name-status <ref>` (árbol de trabajo, rutas relativas a la
/// raíz): mismos archivos que `archeology_scan::git::changed_since`, con
/// el tipo de cambio. None si la ref no es válida o no hay repo.
fn diff_name_status(root: &Path, reference: &str) -> Option<Vec<(&'static str, String)>> {
    // Una ref que empieza por `-` sería una opción de git (`--output=...`).
    if reference.starts_with('-') {
        return None;
    }
    let out = archeology_core::gitcmd::git_command(root)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-status",
            "--relative",
            "-z",
            reference,
            "--",
            ".",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Con `-z`: estado NUL ruta NUL estado NUL ruta…
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut parts = text.split('\0').filter(|s| !s.is_empty());
    let mut files = Vec::new();
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        files.push((change_kind(status), path.to_string()));
    }
    Some(files)
}

fn change_kind(status: &str) -> &'static str {
    match status.chars().next() {
        Some('A') => "added",
        Some('D') => "deleted",
        _ => "modified",
    }
}

/// Archivos cambiados desde el último scan según git (ver el comentario
/// del módulo). None si no hay repositorio git.
fn changed_since_scan(
    project: &Project,
    inv: &archeology_core::model::Inventory,
) -> Option<Vec<(&'static str, String)>> {
    let root = &project.root;
    let scan_time = chrono::DateTime::parse_from_rfc3339(&inv.generated_at)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc));
    // Commit registrado en el scan; si no hay, HEAD.
    let base = inv
        .git
        .as_ref()
        .and_then(|g| g.last_commit.as_deref())
        .and_then(|c| c.split('|').next())
        .filter(|h| !h.is_empty() && !h.starts_with('-'))
        .map(String::from);
    let mut files = match diff_name_status(root, base.as_deref().unwrap_or("HEAD")) {
        Some(f) => f,
        None => {
            // Sin commits todavía: basta con que haya repositorio.
            archeology_core::gitcmd::git_command(root)
                .args(["rev-parse", "--is-inside-work-tree"])
                .output()
                .ok()
                .filter(|o| o.status.success())?;
            Vec::new()
        }
    };
    // Sin seguimiento y no ignorados (relativos al directorio actual = raíz).
    let untracked = archeology_core::gitcmd::git_command(root)
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    for p in String::from_utf8_lossy(&untracked.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
    {
        files.push(("added", p.to_string()));
    }
    // Solo lo modificado después del scan (los borrados no tienen mtime).
    files.retain(|(change, path)| {
        if *change == "deleted" {
            return true;
        }
        let Some(scan) = scan_time else {
            return true;
        };
        fs::metadata(root.join(path))
            .and_then(|m| m.modified())
            .map(|m| chrono::DateTime::<chrono::Utc>::from(m) > scan)
            .unwrap_or(true)
    });
    files.sort_by(|a, b| a.1.cmp(&b.1));
    files.dedup_by(|a, b| a.1 == b.1);
    Some(files)
}

/// Salida de archeology-doc (no es código del proyecto): vault, docs,
/// archivos de contexto y configuración.
fn is_generated(project: &Project, path: &str) -> bool {
    let knowledge = format!("{}/", project.config.paths.knowledge.trim_end_matches('/'));
    let docs = format!("{}/", project.config.paths.docs.trim_end_matches('/'));
    const FILES: &[&str] = &[
        "AGENTS.md",
        "CLAUDE.md",
        "llms.txt",
        archeology_core::config::CONFIG_FILE,
        ".cursor/rules/archeology-doc.mdc",
        ".github/copilot-instructions.md",
    ];
    path.starts_with(&knowledge)
        || path.starts_with(&docs)
        || path.starts_with(".agents/")
        || archeology_core::config::LEGACY_STATE_DIRS
            .iter()
            .any(|d| path.starts_with(&format!("{d}/")))
        || FILES.contains(&path)
}

/// Vuelve a `pending` las tareas `done` cuyo `# Produce` cubre alguna nota.
fn reopen_tasks(
    project: &Project,
    state: &mut State,
    notes: &[String],
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut reopened = Vec::new();
    if notes.is_empty() || !project.tasks_dir().is_dir() {
        return Ok(reopened);
    }
    for e in fs::read_dir(project.tasks_dir())?.flatten() {
        let path = e.path();
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(meta) = planner::parse_task(&text) else {
            continue;
        };
        if meta.status != "done" {
            continue;
        }
        let covers = meta.produce.iter().any(|p| {
            notes
                .iter()
                .any(|n| n == p || (p.ends_with('/') && n.starts_with(p.as_str())))
        });
        if covers {
            if !dry_run {
                archeology_core::fsutil::atomic_write(
                    &path,
                    planner::set_status_in_markdown(&text, "pending").as_bytes(),
                )?;
                state.tasks.insert(meta.task_id.clone(), "pending".into());
                // La tarea dice qué notas re-verificar y por qué.
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                archeology_core::task_context::refresh_task_file(project, state, &path, &today)?;
            }
            reopened.push(meta.task_id);
        }
    }
    reopened.sort();
    Ok(reopened)
}

// ---------------------------------------------------------------------------
// Texto
// ---------------------------------------------------------------------------

/// Marca de cambio: M (modificado), A (añadido), D (borrado).
fn change_mark(change: &str) -> String {
    match change {
        "added" => ui::green("A"),
        "deleted" => ui::red("D"),
        _ => ui::yellow("M"),
    }
}

/// `knowledge/modules/src.md` → `modules/src`.
fn short_note(project: &Project, note: &str) -> String {
    let know = format!("{}/", project.config.paths.knowledge.trim_end_matches('/'));
    note.strip_prefix(&know)
        .unwrap_or(note)
        .trim_end_matches(".md")
        .to_string()
}

fn row(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!(
        "{} {value}\n",
        ui::bold(&ui::pad(label, LABEL_WIDTH))
    ));
}

fn heading(out: &mut String, title: &str) {
    out.push('\n');
    out.push_str(&ui::bold(title));
    out.push('\n');
}

fn more(out: &mut String, total: usize) {
    if total > MAX_LIST {
        out.push_str(&format!(
            "  {}\n",
            ui::dim(&t!("cli.sync.more", count = total - MAX_LIST))
        ));
    }
}

fn render(project: &Project, r: &SyncReport) -> String {
    let mut out = format!(
        "{} {}\n",
        ui::bold("adoc sync"),
        ui::dim(&format!("· {}", project.config.project.name))
    );
    if r.dry_run {
        out.push_str(&ui::yellow(&t!("cli.sync.dry_run_banner")));
        out.push('\n');
    }
    out.push('\n');
    let compared = match &r.compared_against.reference {
        Some(reference) => t!("cli.sync.compared_git", reference = reference),
        None => t!("cli.sync.compared_hashes"),
    };
    row(&mut out, &t!("cli.sync.label.compared"), &compared);

    // Fuentes vigiladas.
    if r.watched.files == 0 {
        row(
            &mut out,
            &t!("cli.sync.label.watched"),
            &ui::yellow(&t!("cli.sync.no_watched")),
        );
    } else {
        row(
            &mut out,
            &t!("cli.sync.label.watched"),
            &t!(
                "cli.sync.watched",
                files = r.watched.files,
                notes = r.watched.notes
            ),
        );
    }

    // Cambios en fuentes documentadas.
    if !r.changed.is_empty() {
        heading(
            &mut out,
            &t!("cli.sync.section.changed", count = r.changed.len()),
        );
        let width = r
            .changed
            .iter()
            .take(MAX_LIST)
            .map(|c| c.path.chars().count())
            .max()
            .unwrap_or(0);
        for c in r.changed.iter().take(MAX_LIST) {
            let mut notes: Vec<String> = c
                .notes
                .iter()
                .take(MAX_NOTES_PER_FILE)
                .map(|n| short_note(project, n))
                .collect();
            if c.notes.len() > MAX_NOTES_PER_FILE {
                notes.push(t!(
                    "cli.sync.more",
                    count = c.notes.len() - MAX_NOTES_PER_FILE
                ));
            }
            out.push_str(&format!(
                "  {} {}  {}\n",
                change_mark(c.change),
                ui::pad(&c.path, width),
                ui::dim(&format!(
                    "→ {}: {}",
                    t!("cli.sync.notes_count", count = c.notes.len()),
                    notes.join(", ")
                ))
            ));
        }
        more(&mut out, r.changed.len());
    }

    // Cambios fuera de la documentación.
    if !r.uncovered.is_empty() {
        heading(
            &mut out,
            &t!("cli.sync.section.uncovered", count = r.uncovered.len()),
        );
        for u in r.uncovered.iter().take(MAX_LIST) {
            out.push_str(&format!("  {} {}\n", change_mark(u.change), u.path));
        }
        more(&mut out, r.uncovered.len());
        out.push_str(&format!("  {}\n", ui::dim(&t!("cli.sync.uncovered_hint"))));
    } else if r.uncovered_method == "unavailable" && r.compared_against.reference.is_none() {
        out.push_str(&format!(
            "{} {}\n",
            ui::bold(&ui::pad(&t!("cli.sync.label.uncovered"), LABEL_WIDTH)),
            ui::dim(&t!("cli.sync.uncovered_unavailable"))
        ));
    }

    // Notas marcadas y tareas reabiertas.
    if !r.stale_notes.is_empty() {
        heading(
            &mut out,
            &t!("cli.sync.section.stale", count = r.stale_notes.len()),
        );
        for n in r.stale_notes.iter().take(MAX_LIST) {
            out.push_str(&format!("  {} {n}\n", ui::yellow("!")));
        }
        more(&mut out, r.stale_notes.len());
    }
    for n in &r.locked_notes {
        out.push_str(&format!(
            "  {}\n",
            ui::dim(&t!("cli.sync.locked", note = n))
        ));
    }
    if !r.reopened_tasks.is_empty() {
        out.push('\n');
        row(
            &mut out,
            &t!("cli.sync.label.reopened"),
            &r.reopened_tasks.join(", "),
        );
    }

    // Resumen.
    out.push('\n');
    if r.changed.is_empty() && r.uncovered.is_empty() {
        out.push_str(&ui::green(&t!("cli.sync.no_changes")));
        out.push('\n');
    } else {
        let parts = [
            t!("cli.sync.sum_changed", count = r.changed.len()),
            t!("cli.sync.sum_stale", count = r.stale_notes.len()),
            t!("cli.sync.sum_reopened", count = r.reopened_tasks.len()),
            t!("cli.sync.sum_uncovered", count = r.uncovered.len()),
        ];
        out.push_str(&parts.join(&ui::dim(" · ")));
        out.push('\n');
    }
    match &r.next_step {
        Some(step) => out.push_str(&format!(
            "{} → {}  {}\n",
            ui::bold(&ui::pad(&t!("cli.sync.label.next"), LABEL_WIDTH)),
            ui::cyan(&ui::bold(&step.command)),
            ui::dim(&step.reason)
        )),
        None => out.push_str(&format!(
            "{} {}\n",
            ui::bold(&ui::pad(&t!("cli.sync.label.next"), LABEL_WIDTH)),
            ui::dim(&t!("cli.sync.next.nothing"))
        )),
    }
    if r.dry_run {
        out.push_str(&ui::dim(&t!("cli.sync.dry_run")));
        out.push('\n');
    }
    out
}
