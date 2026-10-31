//! `context` — contexto documentado de una o varias zonas del código en una
//! sola llamada (para agentes, scripts y la software factory).
//!
//! La lógica vive en `archeology_core::zone`; aquí solo los argumentos, el
//! formato de salida (`context_view`) y los códigos de salida:
//! 0 éxito · 1 uso o proyecto no inicializado · 2 ningún objetivo resuelto ·
//! 3 éxito con notas desactualizadas y `--fail-on-stale`.

use anyhow::Result;
use archeology_core::zone::{self, ContentMode, Options, Section};
use archeology_core::{t, Project};
use clap::Args;

/// Código: ningún objetivo pudo resolverse.
pub const EXIT_UNRESOLVED: i32 = 2;
/// Código: notas desactualizadas con `--fail-on-stale`.
pub const EXIT_STALE: i32 = 3;
/// Código: error de uso (también los errores de clap de este comando).
pub const EXIT_USAGE: i32 = 1;

#[derive(Args)]
pub struct ContextArgs {
    /// Objetivos: ruta (archivo o carpeta), `module:<nombre>`, `feature:<nombre>` o un nombre sin prefijo.
    #[arg(value_name = "TARGET", required_unless_present = "query")]
    targets: Vec<String>,
    /// Búsqueda en títulos y contenido de las notas (se combina con los objetivos).
    #[arg(long)]
    query: Option<String>,
    /// Saltos del grafo para dependencias y notas relacionadas (0 = solo el objetivo).
    #[arg(long, default_value_t = 1)]
    depth: usize,
    /// Secciones a incluir, separadas por comas (por defecto, todas).
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = ["modules", "features", "notes", "findings", "tests", "dependencies", "decisions", "glossary"]
    )]
    include: Vec<String>,
    /// Máximo de notas, por relevancia.
    #[arg(long, default_value_t = 20)]
    max_notes: usize,
    /// Límite total de caracteres del contenido de las notas (0 = sin límite).
    #[arg(long, default_value_t = 0)]
    max_chars: usize,
    /// Contenido de las notas.
    #[arg(long, default_value = "excerpt", value_parser = ["none", "excerpt", "full"])]
    content: String,
    /// Formato de salida (por defecto, text).
    #[arg(long, value_parser = ["text", "json", "markdown"])]
    format: Option<String>,
    /// Alias de `--format json`.
    #[arg(long)]
    json: bool,
    /// Código de salida 3 si alguna nota incluida está desactualizada.
    #[arg(long)]
    fail_on_stale: bool,
    /// Incluir también hallazgos resueltos y descartados.
    #[arg(long)]
    all_findings: bool,
}

/// Ejecuta el comando y devuelve el código de salida.
pub fn run(project: &Project, args: ContextArgs) -> Result<i32> {
    let format = match (args.json, args.format.as_deref()) {
        (true, Some(f)) if f != "json" => {
            eprintln!("error: {}", t!("cli.context.format_conflict", format = f));
            return Ok(EXIT_USAGE);
        }
        (true, _) => "json",
        (false, Some(f)) => f,
        (false, None) => "text",
    };
    let include = if args.include.is_empty() {
        Section::ALL.into_iter().collect()
    } else {
        args.include
            .iter()
            .filter_map(|s| Section::parse(s))
            .collect()
    };
    let opts = Options {
        targets: args.targets,
        query: args.query,
        depth: args.depth,
        include,
        max_notes: args.max_notes,
        max_chars: args.max_chars,
        content: ContentMode::parse(&args.content).unwrap_or(ContentMode::Excerpt),
        all_findings: args.all_findings,
        cwd: std::env::current_dir().ok(),
    };
    let ctx = zone::build(project, &opts);
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&ctx)?),
        "markdown" => print!("{}", super::context_view::markdown(&ctx)),
        _ => print!("{}", super::context_view::text(project, &ctx, &opts)),
    }
    Ok(exit_code(&ctx, args.fail_on_stale))
}

/// Código de salida según el resultado.
fn exit_code(ctx: &zone::Context, fail_on_stale: bool) -> i32 {
    if !ctx.resolved_any() {
        EXIT_UNRESOLVED
    } else if fail_on_stale && ctx.stale_count() > 0 {
        EXIT_STALE
    } else {
        0
    }
}
