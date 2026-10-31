//! archeology-doc: CLI de inspección de proyectos y generación de documentación.
//!
//! Se distribuye con dos binarios equivalentes: `adoc` (corto) y `archeology-doc`.
//!
//! Los textos de ayuda del derive de clap están en inglés (respaldo); la
//! ayuda que se muestra sale del catálogo `cli.help.*` en el idioma de la
//! interfaz (ver [`localized_command`]).

mod commands;
mod ui;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "adoc",
    version,
    about = "Inspects software projects and generates documentation, knowledge and context for AI agents.",
    long_about = None
)]
pub struct Cli {
    /// Operate on a registered project by name or ID (RF-173).
    #[arg(long, global = true)]
    project: Option<String>,

    /// Verbosity (-v info, -vv debug).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Interface language (en|es). Precedence: --lang > ADOC_LANG >
    /// `language` in ~/.archeology-doc/config.toml > system locale > en.
    /// In `init`, also the language of the generated documentation.
    #[arg(long, global = true, value_name = "en|es")]
    lang: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize archeology-doc in the current project.
    Init(commands::init::InitArgs),
    /// User preferences (~/.archeology-doc/config.toml), e.g. the language.
    Config {
        #[command(subcommand)]
        cmd: commands::config::ConfigCmd,
    },
    /// Deterministic inventory of the repository.
    Scan(commands::scan::ScanArgs),
    /// Generate or update the inspection tasks.
    Plan(commands::plan::PlanArgs),
    /// Run tasks with an agent (or export them for manual mode).
    Inspect(commands::inspect::InspectArgs),
    /// Task management (manual mode).
    Task {
        #[command(subcommand)]
        cmd: commands::task::TaskCmd,
    },
    /// Summary of status, findings and progress.
    Review(commands::review::ReviewArgs),
    /// Checklist of what is done and missing, with fix hints (exit code for CI).
    Check(commands::check::CheckArgs),
    /// Detect code changes, mark outdated notes and reopen their tasks.
    Sync(commands::sync::SyncArgs),
    /// VitePress documentation site.
    Site {
        #[command(subcommand)]
        cmd: commands::site::SiteCmd,
    },
    /// List and test agent adapters.
    Agent {
        #[command(subcommand)]
        cmd: commands::agent::AgentCmd,
    },
    /// Management of the global project registry.
    Projects {
        #[command(subcommand)]
        cmd: commands::projects::ProjectsCmd,
    },
    /// Resume the last pending inspection.
    Resume(commands::resume::ResumeArgs),
    /// Show the result: site with dashboard in the browser (or HTML report without VitePress).
    Preview(PreviewArgs),
    /// Generate a single HTML (or Markdown) report with the status, findings and questions.
    Report(commands::report::ReportArgs),
    /// Ask the agent about the project; the answer is saved in the FAQ (knowledge/faq/).
    Ask(commands::ask::AskArgs),
    /// Knowledge graph: opens the interactive HTML in the browser (`--json` prints the JSON).
    Graph(commands::graph::GraphArgs),
    /// Project snapshot: structure, progress, pending tasks and next step (`--format json` for integrations).
    Status(commands::status::StatusArgs),
    /// Documented context of code zones (notes, findings, tests, dependencies) for agents and scripts.
    Context(commands::context_cmd::ContextArgs),
    /// Check requirements: git, Node, VitePress, OpenSpec, agents and configuration.
    Doctor(commands::doctor::DoctorArgs),
    /// Update `adoc` to the latest published version (`--check` only reports).
    Upgrade(commands::upgrade::UpgradeArgs),
    /// Uninstall `adoc`; with --purge also delete ~/.archeology-doc (projects are not touched).
    Uninstall(commands::uninstall::UninstallArgs),
}

#[derive(clap::Args)]
pub struct PreviewArgs {
    /// VitePress server port.
    #[arg(long)]
    pub port: Option<u16>,
    /// Do not open the browser.
    #[arg(long)]
    pub no_open: bool,
    /// Approve running the project's VitePress configuration (repository
    /// JavaScript); remembered for this project.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// Valor de `--lang` en los argumentos, antes de que clap los procese: así
/// el idioma ya está resuelto si clap muestra la ayuda o un error.
fn lang_flag(args: &[std::ffi::OsString]) -> Option<String> {
    let mut it = args.iter().skip(1).map(|a| a.to_string_lossy());
    while let Some(a) = it.next() {
        if a == "--" {
            break;
        }
        if a == "--lang" {
            return it.next().map(|v| v.to_string());
        }
        if let Some(v) = a.strip_prefix("--lang=") {
            return Some(v.to_string());
        }
    }
    None
}

/// Primer argumento que no es una opción global: el subcomando (`context`).
fn subcommand_name(args: &[std::ffi::OsString]) -> Option<String> {
    let mut it = args.iter().skip(1).map(|a| a.to_string_lossy());
    while let Some(a) = it.next() {
        if a == "--project" || a == "--lang" {
            it.next();
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        return Some(a.to_string());
    }
    None
}

/// Punto de entrada compartido por los binarios `adoc` y `archeology-doc`.
pub fn main_entry() -> Result<()> {
    // Idioma de la interfaz: se fija una sola vez, antes de cualquier mensaje.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let flag = lang_flag(&args);
    archeology_core::i18n::init_ui_lang(flag.as_deref());
    let is_context = subcommand_name(&args).as_deref() == Some("context");
    let matches =
        match localized_command(archeology_core::i18n::ui_lang()).try_get_matches_from(args) {
            Ok(m) => m,
            // `adoc context`: un error de uso es el código 1 del contrato (clap
            // usa 2, que en `context` significa "ningún objetivo resuelto").
            Err(e) if is_context && e.use_stderr() => {
                let _ = e.print();
                std::process::exit(commands::context_cmd::EXIT_USAGE);
            }
            Err(e) => e.exit(),
        };
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    // clap y el análisis previo deben coincidir (mismo flag global).
    debug_assert_eq!(cli.lang, flag);
    ui::init(cli.verbose);
    // Windows: restos de una actualización anterior (`adoc.exe.old`).
    if cfg!(windows) {
        commands::upgrade::remove_stale_old_binaries();
    }
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let result = commands::run(cli.command, cli.project);
    // Ctrl+C/SIGTERM durante una corrida: el agente ya se mató, la tarea quedó
    // reanudable y el log escrito. Código 130 como un SIGINT convencional (los
    // bloqueos del proyecto ya se liberaron al volver de `commands::run`).
    if let Err(e) = &result {
        if e.downcast_ref::<archeology_agents::Interrupted>().is_some() {
            eprintln!("{}", archeology_core::t!("cli.main.interrupted"));
            std::process::exit(130);
        }
    }
    result
}

/// Prefijo de las claves de ayuda en el catálogo (`locales/*/cli_help.toml`).
const HELP_PREFIX: &str = "cli.help";

/// Comando de clap con la ayuda (`about`, `long_about`, ayuda de cada
/// argumento, encabezados y flags `--help`/`--version`) en `lang`.
///
/// Se construye el comando del derive, se completa con `build()` (así ya
/// existen los flags de ayuda/versión, el subcomando `help` y los argumentos
/// globales propagados) y se recorre aplicando las claves
/// `cli.help[.<subcomando>...].{about,long_about,arg.<id>}`. Un texto sin clave
/// conserva el del derive (inglés).
pub fn localized_command(lang: archeology_core::i18n::Lang) -> clap::Command {
    let mut cmd = Cli::command();
    cmd.build();
    let globals: Vec<String> = cmd
        .get_arguments()
        .filter(|a| a.is_global_set())
        .map(|a| a.get_id().to_string())
        .collect();
    localize_command(cmd, lang, &[], &globals)
}

/// Texto de `key` si existe en algún idioma (si no, `None`: se deja el del derive).
fn help_text(lang: archeology_core::i18n::Lang, key: &str) -> Option<String> {
    use archeology_core::i18n::{has_key, translate, Lang};
    (has_key(lang, key) || has_key(Lang::FALLBACK, key)).then(|| translate(lang, key, &[]))
}

/// Encabezados de clap (`Str`) solo aceptan `&'static str` sin la feature
/// `string`: el comando se construye una vez por proceso y son pocos textos.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Clave de ayuda de un comando: `cli.help.<ruta>.<sufijo>`.
fn help_key(path: &[String], suffix: &str) -> String {
    let mut key = HELP_PREFIX.to_string();
    for p in path {
        key.push('.');
        key.push_str(p);
    }
    key.push('.');
    key.push_str(suffix);
    key
}

/// Texto propio de clap (`cli.help.clap.<name>`).
fn clap_text(lang: archeology_core::i18n::Lang, name: &str) -> String {
    archeology_core::i18n::translate(lang, &format!("{HELP_PREFIX}.clap.{name}"), &[])
}

/// Localiza un comando y, recursivamente, sus subcomandos. `path` es la ruta
/// de subcomandos desde la raíz; `globals`, los argumentos globales (sus
/// claves son las de la raíz).
fn localize_command(
    mut cmd: clap::Command,
    lang: archeology_core::i18n::Lang,
    path: &[String],
    globals: &[String],
) -> clap::Command {
    use archeology_core::i18n::Lang;
    // Textos que clap trae en inglés: solo se tocan si cambian, así la
    // ayuda en inglés queda idéntica a la de clap.
    let english = |name: &str| clap_text(Lang::En, name);
    let differs = |name: &str| clap_text(lang, name) != english(name);

    if let Some(about) = help_text(lang, &help_key(path, "about")) {
        cmd = cmd.about(about);
        cmd = match help_text(lang, &help_key(path, "long_about")) {
            Some(long) => cmd.long_about(long),
            None => cmd.long_about(None::<&str>),
        };
    }
    if differs("usage") {
        let header = *cmd.get_styles().get_header();
        let usage = clap_text(lang, "usage");
        cmd = cmd.help_template(format!(
            "{{before-help}}{{about-with-newline}}\n{header}{usage}{header:#} {{usage}}\n\n{{all-args}}{{after-help}}"
        ));
    }
    if differs("commands") && cmd.has_subcommands() {
        cmd = cmd.subcommand_help_heading(leak(clap_text(lang, "commands")));
    }

    // Posicionales primero: el orden de los encabezados propios sigue el
    // orden de los argumentos (`mut_arg` los mueve al final).
    let mut ids: Vec<(String, bool)> = cmd
        .get_arguments()
        .map(|a| (a.get_id().to_string(), a.is_positional()))
        .collect();
    ids.sort_by_key(|(_, positional)| !positional);
    let in_help_subcommand = path.last().is_some_and(|p| p == "help");
    for (id, positional) in ids {
        let key_path: &[String] = if globals.contains(&id) { &[] } else { path };
        let text = help_text(lang, &help_key(key_path, &format!("arg.{id}")));
        let heading = if positional { "arguments" } else { "options" };
        let heading_text = differs(heading).then(|| leak(clap_text(lang, heading)));
        cmd = cmd.mut_arg(&id, |mut a| {
            match a.get_action() {
                // Flags propios de clap: `-h/--help` y `-V/--version`.
                clap::ArgAction::Help | clap::ArgAction::HelpShort | clap::ArgAction::HelpLong => {
                    // Con ayuda larga, clap distingue `-h` (resumen) y `--help`.
                    a = if a.get_long_help().is_some() {
                        a.help(clap_text(lang, "help_short"))
                            .long_help(clap_text(lang, "help_long"))
                    } else {
                        a.help(clap_text(lang, "help"))
                    };
                }
                clap::ArgAction::Version => a = a.help(clap_text(lang, "version")),
                _ if in_help_subcommand && id == "subcommand" => {
                    a = a.help(clap_text(lang, "help_subcommand_arg"));
                }
                _ => {
                    if let Some(t) = text {
                        a = a.help(t).long_help(None::<&str>);
                    }
                }
            }
            if let Some(h) = heading_text {
                if a.get_help_heading().is_none() {
                    a = a.help_heading(h);
                }
            }
            a
        });
    }

    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in names {
        if name == "help" {
            // Subcomando `help` de clap: su árbol replica el del comando
            // padre (mismas claves) con la descripción propia de clap.
            let base = path.to_vec();
            cmd = cmd.mut_subcommand(&name, |s| {
                let s = localize_help_subtree(s, lang, &base);
                s.about(clap_text(lang, "help_subcommand"))
            });
            continue;
        }
        let mut sub_path = path.to_vec();
        sub_path.push(name.clone());
        cmd = cmd.mut_subcommand(&name, |s| localize_command(s, lang, &sub_path, globals));
    }
    cmd
}

/// Árbol del subcomando `help` (`adoc help <comando>`): solo nombres y `about`.
fn localize_help_subtree(
    mut cmd: clap::Command,
    lang: archeology_core::i18n::Lang,
    path: &[String],
) -> clap::Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in names {
        let mut sub_path = path.to_vec();
        sub_path.push(name.clone());
        cmd = cmd.mut_subcommand(&name, |s| {
            let s = localize_help_subtree(s, lang, &sub_path);
            match help_text(lang, &help_key(&sub_path, "about")) {
                Some(about) => s.about(about),
                None => s,
            }
        });
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::lang_flag;

    fn args(list: &[&str]) -> Vec<std::ffi::OsString> {
        list.iter().map(|a| a.into()).collect()
    }

    #[test]
    fn lang_flag_is_found_before_clap() {
        assert_eq!(
            lang_flag(&args(&["adoc", "--lang", "en", "scan"])),
            Some("en".into())
        );
        assert_eq!(
            lang_flag(&args(&["adoc", "init", "--lang=es"])),
            Some("es".into())
        );
        assert_eq!(
            lang_flag(&args(&["adoc", "ask", "--", "--lang", "en"])),
            None
        );
        assert_eq!(lang_flag(&args(&["adoc", "scan"])), None);
    }
}

#[cfg(test)]
mod help_tests {
    use super::*;
    use archeology_core::i18n::{has_key, Lang};

    /// Recorre el árbol del derive (sin el subcomando `help` de clap).
    fn walk(cmd: &clap::Command, path: &mut Vec<String>, missing: &mut Vec<String>) {
        for lang in Lang::ALL {
            let key = help_key(path, "about");
            if !has_key(lang, &key) {
                missing.push(format!("{lang}: {key}"));
            }
            if cmd.get_long_about().is_some() && !has_key(lang, &help_key(path, "long_about")) {
                missing.push(format!("{lang}: {}", help_key(path, "long_about")));
            }
            for a in cmd.get_arguments().filter(|a| a.get_help().is_some()) {
                let key = help_key(path, &format!("arg.{}", a.get_id()));
                if !has_key(lang, &key) {
                    missing.push(format!("{lang}: {key}"));
                }
            }
        }
        for sub in cmd.get_subcommands() {
            path.push(sub.get_name().to_string());
            walk(sub, path, missing);
            path.pop();
        }
    }

    #[test]
    fn every_command_and_argument_has_help_in_both_languages() {
        let mut missing = Vec::new();
        walk(&Cli::command(), &mut Vec::new(), &mut missing);
        assert!(missing.is_empty(), "faltan claves de ayuda: {missing:#?}");
    }

    /// El comando localizado sigue siendo válido para clap y para el derive.
    #[test]
    fn localized_command_parses_like_the_derive() {
        for lang in Lang::ALL {
            let cmd = localized_command(lang);
            cmd.clone().debug_assert();
            let m = cmd
                .try_get_matches_from(["adoc", "--lang", "en", "inspect", "--dry-run"])
                .unwrap();
            let cli = Cli::from_arg_matches(&m).unwrap();
            assert_eq!(cli.lang.as_deref(), Some("en"));
            assert!(matches!(cli.command, Commands::Inspect(_)));
        }
    }

    #[test]
    fn help_is_rendered_in_the_requested_language() {
        let render = |lang, args: &[&str]| {
            let err = localized_command(lang)
                .try_get_matches_from(args)
                .unwrap_err();
            err.render().to_string()
        };
        let en = render(Lang::En, &["adoc", "--help"]);
        assert!(en.contains("Usage:") && en.contains("Commands:") && en.contains("Print help"));
        assert!(
            en.contains("Deterministic inventory of the repository"),
            "{en}"
        );
        let es = render(Lang::Es, &["adoc", "--help"]);
        assert!(es.contains("Uso:") && es.contains("Comandos:") && es.contains("Opciones:"));
        assert!(
            es.contains("Inventario determinista del repositorio"),
            "{es}"
        );
        assert!(!es.contains("Usage:") && !es.contains("Print help"), "{es}");
        let es = render(Lang::Es, &["adoc", "task", "answer", "--help"]);
        assert!(
            es.contains("Argumentos:") && es.contains("ID de la tarea"),
            "{es}"
        );
    }
}
