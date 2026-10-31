//! Salidas de `adoc context` para personas (`text`) y para el prompt de un
//! agente (`markdown`), en el idioma de la interfaz. El JSON es el contrato
//! (`archeology_core::zone`); estas vistas solo lo resumen.

use crate::ui;
use archeology_core::zone::{Context, EvidenceCtx, Options};
use archeology_core::{t, Project};
use std::collections::BTreeMap;

/// Elementos que se listan por sección en texto.
const MAX_ITEMS: usize = 8;
/// Ancho máximo de los resúmenes en texto.
const SUMMARY_WIDTH: usize = 100;
/// Orden de severidades.
const SEVERITIES: &[&str] = &["critical", "high", "medium", "low"];

fn section(out: &mut String, title: &str, count: Option<usize>) {
    out.push('\n');
    let title = match count {
        Some(n) => format!("{title} ({n})"),
        None => title.to_string(),
    };
    out.push_str(&ui::bold(&title));
    out.push('\n');
}

fn more(out: &mut String, total: usize) {
    if total > MAX_ITEMS {
        out.push_str(&format!(
            "  {}\n",
            ui::dim(&t!("cli.context.more", count = total - MAX_ITEMS))
        ));
    }
}

fn stale_tag(stale: bool) -> String {
    if stale {
        format!(
            " {}",
            ui::yellow(&format!("[{}]", t!("cli.context.stale_tag")))
        )
    } else {
        String::new()
    }
}

fn severity_color(sev: &str) -> String {
    match sev {
        "critical" | "high" => ui::red(sev),
        "medium" => ui::yellow(sev),
        _ => ui::dim(sev),
    }
}

/// Cita `ruta:línea` o `ruta:a-b`.
fn cite(e: &EvidenceCtx) -> String {
    match (e.line, e.end_line) {
        (Some(a), Some(b)) => format!("{}:{a}-{b}", e.file),
        (Some(a), None) => format!("{}:{a}", e.file),
        _ => e.file.clone(),
    }
}

/// Resumen de texto (terminal).
pub fn text(project: &Project, ctx: &Context, opts: &Options) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}{}{}\n",
        ui::bold(&t!("cli.context.heading")),
        ui::dim(" · "),
        project.config.project.name
    ));

    section(&mut out, &t!("cli.context.section.zone"), None);
    for r in &ctx.resolved_targets {
        let matches = if r.matches.is_empty() {
            String::new()
        } else {
            format!(
                "  {}",
                ui::dim(
                    &r.matches
                        .iter()
                        .take(MAX_ITEMS)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            )
        };
        out.push_str(&format!(
            "  {}{matches}\n",
            t!("cli.context.resolved_as", input = r.input, kind = r.kind)
        ));
    }
    if !ctx.unresolved_targets.is_empty() {
        out.push_str(&format!(
            "  {} {}\n",
            ui::yellow(&t!("cli.context.unresolved")),
            ctx.unresolved_targets.join(", ")
        ));
    }
    if !ctx.resolved_any() {
        out.push_str(&format!("  {}\n", ui::red(&t!("cli.context.no_match"))));
    }

    if !ctx.modules.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.modules"),
            Some(ctx.modules.len()),
        );
        for m in ctx.modules.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  {}  {}{}{}\n",
                ui::cyan(&m.id),
                m.title,
                ui::dim(&format!(" · {}", m.paths.join(", "))),
                stale_tag(m.stale)
            ));
            if !m.summary.is_empty() {
                out.push_str(&format!(
                    "    {}\n",
                    ui::dim(&ui::truncate(&m.summary, SUMMARY_WIDTH))
                ));
            }
        }
        more(&mut out, ctx.modules.len());
    }

    if !ctx.features.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.features"),
            Some(ctx.features.len()),
        );
        for f in ctx.features.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  {}  {}{}\n",
                ui::cyan(&f.id),
                f.title,
                stale_tag(f.stale)
            ));
        }
        more(&mut out, ctx.features.len());
    }

    if !ctx.findings.is_empty() {
        let mut by_sev: BTreeMap<usize, usize> = BTreeMap::new();
        for f in &ctx.findings {
            let rank = SEVERITIES
                .iter()
                .position(|s| *s == f.severity)
                .unwrap_or(SEVERITIES.len());
            *by_sev.entry(rank).or_insert(0) += 1;
        }
        let counts: Vec<String> = by_sev
            .iter()
            .map(|(r, n)| format!("{} {n}", SEVERITIES.get(*r).copied().unwrap_or("?")))
            .collect();
        section(
            &mut out,
            &t!("cli.context.section.findings"),
            Some(ctx.findings.len()),
        );
        out.push_str(&format!("  {}\n", ui::dim(&counts.join(" · "))));
        for f in ctx.findings.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  [{}] {}  {}{}\n",
                severity_color(&f.severity),
                f.title,
                ui::dim(&format!("{} · {}", f.category, f.note)),
                stale_tag(f.stale)
            ));
        }
        more(&mut out, ctx.findings.len());
    }

    let tests = &ctx.tests;
    section(&mut out, &t!("cli.context.section.tests"), None);
    if tests.covered.is_empty() && tests.missing.is_empty() && tests.disabled_or_empty.is_empty() {
        out.push_str(&format!("  {}\n", ui::dim(&t!("cli.context.tests_none"))));
    } else {
        out.push_str(&format!(
            "  {} · {} · {}\n",
            ui::green(&t!("cli.context.covered", count = tests.covered.len())),
            if tests.missing.is_empty() {
                t!("cli.context.missing", count = 0)
            } else {
                ui::yellow(&t!("cli.context.missing", count = tests.missing.len()))
            },
            t!(
                "cli.context.disabled",
                count = tests.disabled_or_empty.len()
            )
        ));
        for m in tests.missing.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  - {} {}\n",
                t!("cli.context.scenario_missing", scenario = m.scenario),
                ui::dim(&format!("({})", m.feature))
            ));
        }
        more(&mut out, tests.missing.len());
        for d in tests.disabled_or_empty.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  - {} {}\n",
                d.file,
                ui::dim(&format!("({})", d.reason))
            ));
        }
    }

    let deps = &ctx.dependencies;
    if !deps.outgoing.is_empty() || !deps.incoming.is_empty() {
        section(&mut out, &t!("cli.context.section.dependencies"), None);
        for (label, list, target) in [
            (t!("cli.context.outgoing"), &deps.outgoing, true),
            (t!("cli.context.incoming"), &deps.incoming, false),
        ] {
            if list.is_empty() {
                continue;
            }
            let mut names: Vec<String> = list
                .iter()
                .take(MAX_ITEMS)
                .map(|e| if target { e.to.clone() } else { e.from.clone() })
                .collect();
            names.dedup();
            if list.len() > MAX_ITEMS {
                names.push(t!("cli.context.more", count = list.len() - MAX_ITEMS));
            }
            out.push_str(&format!(
                "  {} {}\n",
                ui::dim(&ui::pad(&label, 9)),
                names.join(", ")
            ));
        }
    }

    if !ctx.notes.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.notes"),
            Some(ctx.notes.len()),
        );
        for n in ctx.notes.iter().take(MAX_ITEMS) {
            let invalid = n.evidence.iter().filter(|e| !e.valid).count();
            let invalid = if invalid > 0 {
                format!(
                    " {}",
                    ui::red(&format!("[{}]", t!("cli.context.invalid_evidence")))
                )
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  {}  {}  {}{}{}\n",
                ui::dim(&format!("{:.2}", n.relevance)),
                n.title,
                ui::dim(&n.path),
                stale_tag(n.stale),
                invalid
            ));
        }
        more(&mut out, ctx.notes.len());
    }

    if !ctx.decisions.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.decisions"),
            Some(ctx.decisions.len()),
        );
        for d in ctx.decisions.iter().take(MAX_ITEMS) {
            out.push_str(&format!("  {}  {}\n", d.title, ui::dim(&d.note)));
        }
    }
    if !ctx.glossary.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.glossary"),
            Some(ctx.glossary.len()),
        );
        for g in ctx.glossary.iter().take(MAX_ITEMS) {
            out.push_str(&format!(
                "  {}: {}\n",
                ui::bold(&g.term),
                ui::truncate(&g.definition, SUMMARY_WIDTH)
            ));
        }
    }
    if !ctx.open_questions.is_empty() {
        section(
            &mut out,
            &t!("cli.context.section.questions"),
            Some(ctx.open_questions.len()),
        );
        for q in ctx.open_questions.iter().take(MAX_ITEMS) {
            out.push_str(&format!("  {}  {}\n", ui::cyan(&q.id), q.question));
        }
    }
    if !ctx.warnings.is_empty() {
        section(&mut out, &t!("cli.context.section.warnings"), None);
        for w in &ctx.warnings {
            out.push_str(&format!("  {} {w}\n", ui::yellow("!")));
        }
    }

    // Siguiente paso.
    let first = ctx
        .resolved_targets
        .iter()
        .find(|r| r.kind != "query")
        .map(|r| r.input.clone())
        .or_else(|| opts.targets.first().cloned())
        .unwrap_or_default();
    let undocumented =
        ctx.resolved_any() && ctx.notes.is_empty() && ctx.modules.iter().all(|m| m.note.is_none());
    let next = if undocumented {
        t!("cli.context.next_inspect")
    } else if ctx.stale_count() > 0 {
        t!("cli.context.next_sync")
    } else {
        t!("cli.context.next_json", target = first)
    };
    section(&mut out, &t!("cli.context.next"), None);
    out.push_str(&format!("  {} {}\n", ui::cyan("→"), next));
    out
}

/// Bloque Markdown para el prompt de un agente: avisos de notas
/// desactualizadas al inicio, una sección por tipo y citas `ruta:línea`.
pub fn markdown(ctx: &Context) -> String {
    let mut out = String::new();
    let targets: Vec<String> = ctx
        .resolved_targets
        .iter()
        .map(|r| format!("`{}`", r.input))
        .collect();
    out.push_str(&format!(
        "# {}: {}\n\n",
        t!("cli.context.heading"),
        if targets.is_empty() {
            t!("cli.context.none")
        } else {
            targets.join(", ")
        }
    ));

    // Avisos primero: el agente debe saber qué notas no son fiables.
    let stale: Vec<String> = ctx
        .notes
        .iter()
        .filter(|n| n.stale)
        .map(|n| {
            format!(
                "> - `{}`: {}",
                n.path,
                n.stale_reason.clone().unwrap_or_default()
            )
        })
        .collect();
    if !ctx.warnings.is_empty() || !stale.is_empty() {
        out.push_str(&format!(
            "> **{}**\n>\n",
            t!("cli.context.section.warnings")
        ));
        for w in &ctx.warnings {
            out.push_str(&format!("> - {w}\n"));
        }
        for s in &stale {
            out.push_str(s);
            out.push('\n');
        }
        out.push('\n');
    }
    if !ctx.unresolved_targets.is_empty() {
        out.push_str(&format!(
            "**{}:** {}\n\n",
            t!("cli.context.unresolved"),
            ctx.unresolved_targets
                .iter()
                .map(|u| format!("`{u}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if !ctx.modules.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.modules")));
        for m in &ctx.modules {
            let paths: Vec<String> = m.paths.iter().map(|p| format!("`{p}`")).collect();
            out.push_str(&format!(
                "- **{}** — {} ({})",
                m.id,
                m.title,
                paths.join(", ")
            ));
            if !m.summary.is_empty() {
                out.push_str(&format!(": {}", m.summary));
            }
            if let Some(n) = &m.note {
                out.push_str(&format!(" [`{n}`]"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    if !ctx.features.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.features")));
        for f in &ctx.features {
            out.push_str(&format!("- **{}** — {}", f.id, f.title));
            if !f.summary.is_empty() {
                out.push_str(&format!(": {}", f.summary));
            }
            out.push_str(&format!(" [`{}`]\n", f.note));
        }
        out.push('\n');
    }
    if !ctx.findings.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.findings")));
        for f in &ctx.findings {
            let ev: Vec<String> = f
                .evidence
                .iter()
                .map(|e| {
                    if e.valid {
                        format!("`{}`", cite(e))
                    } else {
                        format!("`{}` ({})", cite(e), t!("cli.context.invalid_evidence"))
                    }
                })
                .collect();
            out.push_str(&format!(
                "- **{}** · {} — {} (`{}`): {}",
                f.severity, f.category, f.title, f.id, f.summary
            ));
            if !ev.is_empty() {
                out.push_str(&format!(" — {}", ev.join(", ")));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    let tests = &ctx.tests;
    if !tests.covered.is_empty() || !tests.missing.is_empty() || !tests.disabled_or_empty.is_empty()
    {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.tests")));
        for c in &tests.covered {
            let refs: Vec<String> = c.tests.iter().map(|t| format!("`{t}`")).collect();
            out.push_str(&format!("- ✓ {} ({})", c.scenario, c.feature));
            if !refs.is_empty() {
                out.push_str(&format!(" — {}", refs.join(", ")));
            }
            out.push('\n');
        }
        for m in &tests.missing {
            out.push_str(&format!(
                "- ✗ {} ({})\n",
                t!("cli.context.scenario_missing", scenario = m.scenario),
                m.feature
            ));
        }
        for d in &tests.disabled_or_empty {
            out.push_str(&format!("- ⚠ `{}` ({})\n", d.file, d.reason));
        }
        out.push('\n');
    }
    let deps = &ctx.dependencies;
    if !deps.outgoing.is_empty() || !deps.incoming.is_empty() {
        out.push_str(&format!(
            "## {}\n\n",
            t!("cli.context.section.dependencies")
        ));
        for e in deps.outgoing.iter().chain(&deps.incoming) {
            out.push_str(&format!(
                "- {} → {} ({}, {})\n",
                e.from,
                e.to,
                e.kinds.join("/"),
                e.depth
            ));
        }
        out.push('\n');
    }
    if !ctx.notes.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.notes")));
        for n in &ctx.notes {
            out.push_str(&format!(
                "### {} (`{}`, {})\n\n",
                n.title,
                n.path,
                t!(
                    "cli.context.relevance",
                    value = format!("{:.2}", n.relevance)
                )
            ));
            if n.stale {
                out.push_str(&format!(
                    "> ⚠ {}: {}\n\n",
                    t!("cli.context.stale_tag"),
                    n.stale_reason.clone().unwrap_or_default()
                ));
            }
            if !n.content.is_empty() {
                out.push_str(n.content.trim());
                out.push_str("\n\n");
            }
            if !n.evidence.is_empty() {
                let ev: Vec<String> = n
                    .evidence
                    .iter()
                    .map(|e| {
                        if e.valid {
                            format!("`{}`", cite(e))
                        } else {
                            format!("`{}` ({})", cite(e), t!("cli.context.invalid_evidence"))
                        }
                    })
                    .collect();
                out.push_str(&format!(
                    "{}: {}\n\n",
                    t!("cli.context.evidence"),
                    ev.join(", ")
                ));
            }
        }
    }
    if !ctx.decisions.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.decisions")));
        for d in &ctx.decisions {
            out.push_str(&format!(
                "- **{}** (`{}`): {}\n",
                d.title, d.note, d.summary
            ));
        }
        out.push('\n');
    }
    if !ctx.glossary.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.glossary")));
        for g in &ctx.glossary {
            out.push_str(&format!(
                "- **{}**: {} (`{}`)\n",
                g.term, g.definition, g.note
            ));
        }
        out.push('\n');
    }
    if !ctx.open_questions.is_empty() {
        out.push_str(&format!("## {}\n\n", t!("cli.context.section.questions")));
        for q in &ctx.open_questions {
            out.push_str(&format!(
                "- **{}**: {} ({})\n",
                q.id,
                q.question,
                q.related.join(", ")
            ));
        }
        out.push('\n');
    }
    out
}
