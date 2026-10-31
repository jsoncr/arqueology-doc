//! Smoke tests de punta a punta sobre proyectos temporales.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

/// Proyecto temporal con su propio directorio de configuración global.
struct Sandbox {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    config: std::path::PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let config = tmp.path().join("config");
        fs::create_dir_all(&root).unwrap();
        Self {
            _tmp: tmp,
            root,
            config,
        }
    }

    fn cmd(&self) -> Command {
        let mut cmd = adoc_cmd("archeology-doc", &self.config);
        cmd.current_dir(&self.root);
        cmd
    }

    /// Como [`Self::cmd`], sin idioma fijado: sin `ADOC_LANG` ni variables
    /// de locale salvo `LANG=C` (resolución determinista).
    fn cmd_without_lang(&self) -> Command {
        let mut cmd = self.cmd();
        cmd.env_remove("ADOC_LANG")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", "C");
        cmd
    }

    /// Espacio de trabajo global del proyecto (`<config>/projects/<nombre>-<id8>`).
    fn work(&self) -> std::path::PathBuf {
        let mut dirs: Vec<_> = fs::read_dir(self.config.join("projects"))
            .expect("sin espacio de trabajo: ¿se ejecutó init?")
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        dirs.pop().unwrap()
    }

    fn run(&self, args: &[&str]) -> String {
        let out = self.cmd().args(args).assert().success();
        String::from_utf8_lossy(&out.get_output().stdout).to_string()
    }
}

/// Binario `bin` con el registro global aislado (`config`) y la interfaz en
/// español (`ADOC_LANG=es`): las aserciones de estos tests usan los textos
/// en español. Los tests de idioma lo sobrescriben.
fn adoc_cmd(bin: &str, config: &Path) -> Command {
    let mut cmd = Command::cargo_bin(bin).unwrap();
    // Registro global aislado: nunca tocar el del usuario.
    cmd.env("ARCHEOLOGY_DOC_CONFIG_DIR", config)
        .env("ADOC_LANG", "es")
        // `adoc doctor` no consulta la última versión publicada (sin red).
        .env("ADOC_NO_UPDATE_CHECK", "1");
    cmd
}

fn fixture(dir: &Path) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/main.py"),
        "def main():\n    pass  # TODO: implementar\n",
    )
    .unwrap();
    fs::write(dir.join("README.md"), "# demo\n").unwrap();
}

#[test]
fn init_scan_plan_check_flow() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    let root = &sb.root;

    sb.run(&["init", "--yes"]);
    assert!(root.join("archeology-doc.toml").is_file());
    assert!(root.join("AGENTS.md").is_file());
    assert!(root.join("knowledge/_templates/feature.md").is_file());
    assert!(sb.config.join("registry.toml").is_file());

    sb.run(&["scan"]);
    assert!(sb.work().join("inventory.json").is_file());
    // El proyecto solo contiene resultados: el estado vive fuera.
    assert!(!root.join(".archeology-doc").exists());
    assert!(root.join("knowledge/00-overview.md").is_file());

    sb.run(&["plan"]);
    assert!(sb.work().join("tasks/T-001.md").is_file());

    assert!(sb.run(&["task", "list"]).contains("T-001"));
    assert!(sb.run(&["inspect", "--dry-run"]).contains("T-001"));
    sb.run(&["check"]);
}

#[test]
fn scan_excludes_own_output_and_sees_ci_and_root_tests() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    fs::create_dir_all(sb.root.join(".github/workflows")).unwrap();
    fs::write(sb.root.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
    fs::create_dir_all(sb.root.join("tests")).unwrap();
    fs::write(sb.root.join("tests/check_main.py"), "def t(): pass\n").unwrap();
    fs::write(sb.root.join(".env"), "SECRET=1\n").unwrap();

    sb.run(&["init", "--yes"]);
    let inv: serde_json::Value = serde_json::from_str(&sb.run(&["scan", "--json"])).unwrap();

    let list = |k: &str| -> Vec<String> {
        inv[k]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(list("ci_files"), vec![".github/workflows/ci.yml"]);
    assert!(list("test_files").contains(&"tests/check_main.py".to_string()));
    // La salida de archeology-doc y los secretos no forman parte del inventario.
    let docs = list("doc_files");
    assert!(!docs
        .iter()
        .any(|d| d.starts_with("docs/") || d == "AGENTS.md"));
    let modules: Vec<&str> = inv["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(!modules.contains(&"docs"));
    assert!(!modules.contains(&"knowledge"));
    assert!(!inv.to_string().contains(".env"));
}

/// Agente falso: escribe una nota válida por cada entrada de `# Produce`.
#[cfg(unix)]
const FAKE_AGENT: &str = r#"
f="$1"
awk '/^# Produce/{p=1;next} /^# /{p=0} p && /^- /{l=substr($0,3); sub(/ \(.*\)$/, "", l); gsub(/`/, "", l); print l}' "$f" |
while read -r out; do
  case "$out" in
    */) mkdir -p "$out"; n="${out}note-$(basename "$f" .md).md" ;;
    *) mkdir -p "$(dirname "$out")"; n="$out" ;;
  esac
  case "$n" in
    knowledge/*) printf -- '---\nid: %s\ntype: module\ntitle: t\nsources:\n  - README.md:1\n---\n' "$(echo "$n" | tr '/.' '--')" > "$n" ;;
    *) echo "x" >> "$n" ;;
  esac
done
"#;

#[cfg(unix)]
fn add_adapter(sb: &Sandbox, name: &str, script: &str) {
    let root = &sb.root;
    let mut cfg = fs::read_to_string(root.join("archeology-doc.toml")).unwrap();
    let script = script
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    cfg.push_str(&format!(
        "\n[agent.adapters.{name}]\ncommand = \"sh\"\nargs = [\"-c\", \"{script}\", \"sh\", \"{{prompt_file}}\"]\n"
    ));
    fs::write(root.join("archeology-doc.toml"), cfg).unwrap();
    // Un adaptador propio no se ejecuta sin aprobación.
    sb.run(&["agent", "trust", name, "--yes"]);
}

#[cfg(unix)]
#[test]
fn inspect_runs_all_dependency_layers_in_one_run() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let out = sb.run(&["inspect", "--agent", "fake"]);
    assert!(!out.contains("falló"), "{out}");
    let list = sb.run(&["task", "list"]);
    assert!(!list.contains("pending"), "{list}");
    assert!(sb
        .root
        .join("knowledge/features")
        .read_dir()
        .unwrap()
        .next()
        .is_some());
    sb.run(&["check"]);
    // Log legible por corrida (RNF-09), en el espacio de trabajo.
    let run = fs::read_dir(sb.work().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let log = fs::read_to_string(run.path().join("run.log")).unwrap();
    assert!(log.contains("T-001 done"), "{log}");
    // Tiempos para el dashboard: última ejecución por tarea y totales.
    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    assert_eq!(st["tasks"][0]["last_run"]["status"], "done");
    assert_eq!(st["tasks"][0]["last_run"]["attempts"], 1);
    assert_eq!(st["timing"]["runs"], 1);
    assert!(st["timing"]["avg_done_ms"].is_number(), "{}", st["timing"]);
    // Datos del proyecto (inventario) para el dashboard.
    let inv = &st["inventory"];
    assert!(inv["total_files"].as_u64().unwrap() >= 2, "{inv}");
    assert!(inv["total_lines"].as_u64().unwrap() > 0, "{inv}");
    assert!(inv["languages"][0]["name"].is_string(), "{inv}");
}

#[cfg(unix)]
#[test]
fn inspect_fails_task_without_output_and_blocks_dependents() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    add_adapter(&sb, "lazy", "true");
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let out = sb.run(&["inspect", "--agent", "lazy"]);
    assert!(out.contains("✗ T-001"), "{out}");
    assert!(out.contains("⏸ T-002 espera T-001"), "{out}");
    assert!(!sb.run(&["task", "list"]).contains(" done "));
}

#[test]
fn task_done_requires_valid_output() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "manual"]);

    // T-002 produce knowledge/modules/src.md, que aún no existe.
    sb.cmd().args(["task", "done", "T-002"]).assert().failure();
    fs::write(
        sb.root.join("knowledge/modules/src.md"),
        "---\nid: mod-src\ntype: module\ntitle: src\nsources:\n  - src/main.py:1\n---\n",
    )
    .unwrap();
    sb.run(&["task", "done", "T-002"]);
}

#[test]
fn commands_fail_without_project() {
    let sb = Sandbox::new();
    sb.cmd().arg("scan").assert().failure();
}

#[test]
fn sync_marks_notes_stale_and_reopens_tasks() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "manual"]);

    let note = sb.root.join("knowledge/modules/src.md");
    fs::write(
        &note,
        "---\nid: mod-src\ntype: module\ntitle: src\nstatus: draft\nsources:\n  - src/main.py:1-2\n---\n",
    )
    .unwrap();
    let locked = sb.root.join("knowledge/decisions/keep.md");
    let locked_text =
        "---\nid: dec-keep\ntype: decision\nstatus: reviewed\nlocked: true\nsources:\n  - src/main.py:1\n---\n";
    fs::write(&locked, locked_text).unwrap();
    sb.run(&["task", "done", "T-002"]);
    assert!(sb.run(&["sync"]).contains("Sin cambios"));

    fs::write(sb.root.join("src/main.py"), "def main():\n    return 1\n").unwrap();
    let out = sb.run(&["sync"]);
    assert!(out.contains("Notas marcadas obsoletas (1)"), "{out}");
    assert!(out.contains("! knowledge/modules/src.md"), "{out}");
    assert!(out.contains("M src/main.py"), "{out}");
    assert!(
        out.contains("Tareas reabiertas") && out.contains("T-002"),
        "{out}"
    );
    assert!(
        out.contains("bloqueada, no se modifica: knowledge/decisions/keep.md"),
        "{out}"
    );
    assert!(out.contains("→ adoc inspect"), "{out}");
    assert!(fs::read_to_string(&note).unwrap().contains("status: stale"));
    assert_eq!(fs::read_to_string(&locked).unwrap(), locked_text);
    assert!(sb
        .run(&["task", "list", "--status", "pending"])
        .contains("T-002"));

    // Un cambio se notifica una sola vez; `check --strict` falla con notas stale.
    assert!(sb.run(&["sync"]).contains("Sin cambios"));
    sb.run(&["check"]);
    sb.cmd().args(["check", "--strict"]).assert().failure();
}

#[test]
fn scan_is_idempotent_and_preserves_overview_edits() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    sb.run(&["scan"]);
    let overview = sb.root.join("knowledge/00-overview.md");
    let mut text = fs::read_to_string(&overview).unwrap();
    text.push_str("\n## Arquitectura\n\nTexto del agente.\n");
    fs::write(&overview, &text).unwrap();

    sb.run(&["scan"]);
    assert_eq!(fs::read_to_string(&overview).unwrap(), text);
}

#[test]
fn check_rejects_out_of_range_evidence() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    fs::write(
        sb.root.join("knowledge/features/f.md"),
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:40-50\n---\n",
    )
    .unwrap();
    let out = sb.cmd().arg("check").assert().failure();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(out.contains("✗ Evidencias"), "{out}");
    assert!(
        out.contains("knowledge/features/f.md — evidencia fuera de rango"),
        "{out}"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &dest);
        } else {
            fs::copy(&p, &dest).unwrap();
        }
    }
}

fn fixture_dir(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// Sin `adoc spec` (desactivado temporalmente), el cumplimiento de OpenSpec
/// sigue en `review`, `status` y `check`.
#[cfg(unix)]
#[test]
fn openspec_compliance_is_reported_without_spec_command() {
    let sb = Sandbox::new();
    copy_dir(&fixture_dir("with-openspec"), &sb.root);
    // Cambio sin actividad desde 2020 (sembrado como abandonado).
    std::process::Command::new("sh")
        .args([
            "-c",
            "find openspec/changes/add-export -exec touch -t 202001010000 {} +",
        ])
        .current_dir(&sb.root)
        .status()
        .unwrap();
    sb.run(&["init", "--yes"]);

    fs::write(
        sb.root.join("knowledge/specs/billing.md"),
        "---\nid: spec-billing\ntype: spec-report\ntitle: billing\ncapability: billing\n\
         requirements:\n  - id: billing/add-numbers\n    status: implemented\n    evidence: [src/main.py:4]\n\
         \x20 - id: billing/round-totals\n    status: contradicted\n    evidence: [src/main.py:8-10]\n\
         \x20 - id: billing/apply-discount\n    status: not-found\n---\n",
    )
    .unwrap();
    let review: serde_json::Value =
        serde_json::from_str(&sb.run(&["review", "--format", "json"])).unwrap();
    assert_eq!(review["openspec"]["by_status"]["contradicted"], 1);
    assert_eq!(review["openspec"]["stale_changes"][0], "add-export");

    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    assert_eq!(st["specs"]["totals"]["contradicted"], 1, "{st}");

    let out = sb.run(&["check"]);
    assert!(out.contains("! OpenSpec"), "{out}");
    assert!(out.contains("posiblemente abandonado"), "{out}");
}

/// `adoc spec` ya no es un comando (desactivado mientras se rediseña).
#[test]
fn spec_is_no_longer_a_command() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let out = sb.cmd().args(["spec", "status"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("spec"), "{err}");
    let help = sb.run(&["--help"]);
    assert!(!help.contains("\n  spec "), "{help}");
    sb.cmd().args(["help", "spec"]).assert().failure();
}

#[test]
fn projects_list_shows_progress() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--name", "demo"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let rows: serde_json::Value =
        serde_json::from_str(&sb.run(&["projects", "list", "--format", "json"])).unwrap();
    assert_eq!(rows[0]["name"], "demo");
    assert_eq!(rows[0]["progress"], 0);
    assert_eq!(rows[0]["open_findings"], 0);
}

#[test]
fn site_sync_builds_sidebar_wiki_and_llms_index() {
    let sb = Sandbox::new();
    copy_dir(&fixture_dir("with-openspec"), &sb.root);
    sb.run(&["init", "--yes"]);
    assert!(sb.root.join("docs/.vitepress/config.mts").is_file());
    fs::write(
        sb.root.join("knowledge/features/billing.md"),
        "---\nid: feat-billing\ntype: feature\ntitle: Facturación\nsources: [src/main.py:4]\n---\n\
         Ver [[facts|hechos]] y [[nada]]. Evidencia [main](../../src/main.py).\n",
    )
    .unwrap();
    sb.run(&["site", "sync"]);

    let sidebar =
        fs::read_to_string(sb.root.join("docs/.vitepress/sidebar.generated.json")).unwrap();
    assert!(sidebar.contains("/wiki/features/billing"), "{sidebar}");
    assert!(sidebar.contains("/specs/billing"), "{sidebar}");
    let page = fs::read_to_string(sb.root.join("docs/wiki/features/billing.md")).unwrap();
    assert!(page.contains("[hechos](/wiki/memory/facts)"), "{page}");
    assert!(page.contains("main (`../../src/main.py`)"), "{page}");
    let llms = fs::read_to_string(sb.root.join("llms.txt")).unwrap();
    assert!(
        llms.contains("[Facturación](knowledge/features/billing.md)"),
        "{llms}"
    );
    // La copia del vault no se inventaría como código.
    let inv = sb.run(&["scan", "--json"]);
    assert!(!inv.contains("docs/wiki"));
}

/// Build real: `adoc site setup` instala VitePress en el directorio de
/// configuración del sandbox y `check --site` compila (requiere Node y red:
/// `cargo test -- --ignored`).
#[test]
#[ignore]
fn check_site_builds_with_shared_vitepress() {
    let sb = Sandbox::new();
    copy_dir(&fixture_dir("with-openspec"), &sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["site", "setup"]);
    sb.run(&["check", "--site", "--yes"]);
    assert!(sb.root.join("docs/.vitepress/dist/index.html").is_file());
}

/// VitePress falso en la instalación compartida: verifica la resolución,
/// el enlace `docs/node_modules` y el `.gitignore` sin red.
#[cfg(unix)]
#[test]
fn site_build_uses_shared_vitepress_install() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("node no disponible; se omite");
        return;
    }
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let pkg = sb.config.join("vitepress/node_modules/vitepress");
    fs::create_dir_all(pkg.join("bin")).unwrap();
    fs::write(
        pkg.join("package.json"),
        r#"{"name":"vitepress","version":"9.9.9","bin":{"vitepress":"bin/vitepress.js"}}"#,
    )
    .unwrap();
    fs::write(
        pkg.join("bin/vitepress.js"),
        "require('fs').writeFileSync('built.txt', process.argv.slice(2).join(' '))\n",
    )
    .unwrap();

    // Ejecuta JavaScript del repositorio: sin terminal ni --yes, error con instrucciones.
    let out = sb.cmd().args(["site", "build"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("--yes"), "{err}");
    assert!(!sb.root.join("built.txt").exists());
    sb.run(&["site", "build", "--yes"]);
    fs::remove_file(sb.root.join("built.txt")).unwrap();
    // La aprobación se recuerda para el proyecto.
    sb.run(&["site", "build"]);
    let built = fs::read_to_string(sb.root.join("built.txt")).unwrap();
    assert!(built.starts_with("build "), "{built}");
    let link = fs::read_link(sb.root.join("docs/node_modules")).unwrap();
    assert_eq!(link, sb.config.join("vitepress/node_modules"));
    assert!(fs::read_to_string(sb.root.join("docs/.gitignore"))
        .unwrap()
        .contains("node_modules/"));

    let doctor: serde_json::Value =
        serde_json::from_str(&sb.run(&["doctor", "--format", "json"])).unwrap();
    let vp = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "vitepress")
        .unwrap();
    assert_eq!(vp["level"], "ok");
    assert!(vp["detail"]
        .as_str()
        .unwrap()
        .contains("9.9.9 (compartida)"));
}

#[test]
fn doctor_reports_invalid_config_and_adoc_alias_works() {
    let sb = Sandbox::new();
    fs::write(
        sb.root.join("archeology-doc.toml"),
        "[project]\nid = \"\"\nname = \"x\"\n",
    )
    .unwrap();
    let mut cmd = adoc_cmd("adoc", &sb.config);
    cmd.current_dir(&sb.root)
        .args(["doctor", "--format", "json"]);
    let out = cmd.assert().failure();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert!(json.to_string().contains("archeology-doc.toml inválido"));
}

#[test]
fn scan_writes_agent_context_adapters_and_skills() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    fs::write(
        sb.root.join("package.json"),
        r#"{"scripts":{"test":"node --test","lint":"eslint ."}}"#,
    )
    .unwrap();
    fs::write(sb.root.join("AGENTS.md"), "# Mío\n\nTexto del equipo.\n").unwrap();
    sb.run(&[
        "init",
        "--yes",
        "--agent",
        "manual",
        "--adapters",
        "claude,cursor,copilot",
    ]);
    sb.run(&["scan"]);

    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    assert!(
        agents.starts_with("# Mío\n\nTexto del equipo.\n"),
        "{agents}"
    );
    assert!(agents.contains("- test: `npm test`"), "{agents}");
    assert!(agents.contains("archeology-doc:begin context"));
    assert!(agents.contains("`.env*`"));
    assert!(fs::read_to_string(sb.root.join("CLAUDE.md"))
        .unwrap()
        .contains("@AGENTS.md"));
    assert!(
        fs::read_to_string(sb.root.join(".cursor/rules/archeology-doc.mdc"))
            .unwrap()
            .starts_with("---\ndescription:")
    );
    assert!(sb.root.join(".github/copilot-instructions.md").is_file());
    let skill = fs::read_to_string(sb.root.join(".agents/skills/run-tests/SKILL.md")).unwrap();
    assert!(
        skill.contains("name: run-tests") && skill.contains("npm test"),
        "{skill}"
    );
    assert!(sb
        .root
        .join(".agents/skills/project-memory/SKILL.md")
        .is_file());
    assert!(sb.root.join("knowledge/_templates/session.md").is_file());
    assert!(
        fs::read_to_string(sb.root.join("knowledge/memory/facts.md"))
            .unwrap()
            .contains("memory: stable")
    );

    // Idempotente: un segundo scan no cambia nada.
    sb.run(&["scan"]);
    assert_eq!(
        fs::read_to_string(sb.root.join("AGENTS.md")).unwrap(),
        agents
    );
    sb.run(&["check"]);
}

#[test]
fn mocs_and_duplicate_findings() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    let finding = |id: &str| {
        format!(
            "---\nid: {id}\ntype: finding\ntitle: {id}\nkind: risk\nseverity: high\nstate: open\n\
             sources:\n  - src/main.py:2\n---\n"
        )
    };
    fs::write(sb.root.join("knowledge/findings/a.md"), finding("find-a")).unwrap();
    fs::write(sb.root.join("knowledge/findings/b.md"), finding("find-b")).unwrap();
    sb.run(&["scan"]);

    let moc = fs::read_to_string(sb.root.join("knowledge/findings/index.md")).unwrap();
    assert!(
        moc.contains("- [[findings/a|find-a]] — high · open"),
        "{moc}"
    );
    let out = sb.cmd().arg("check").assert().success();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(out.contains("! Duplicados"), "{out}");
    assert!(out.contains("posible hallazgo duplicado"), "{out}");

    // Mismo id en dos notas: error.
    fs::write(
        sb.root.join("knowledge/findings/c.md"),
        finding("find-a").replace("risk", "config"),
    )
    .unwrap();
    sb.cmd().arg("check").assert().failure();
}

/// Flujo del MVP sobre el fixture mediano: inventario, plan acotado con
/// pistas de contradicciones y ejecución completa con un agente falso.
#[cfg(unix)]
#[test]
fn medium_fixture_end_to_end() {
    let sb = Sandbox::new();
    copy_dir(&fixture_dir("medium-ts"), &sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    fs::write(
        sb.root.join("archeology-doc.toml"),
        cfg.replace("max_files_per_task = 40", "max_files_per_task = 5"),
    )
    .unwrap();

    let inv: serde_json::Value = serde_json::from_str(&sb.run(&["scan", "--json"])).unwrap();
    let text = inv.to_string();
    for expected in [
        "src/server.ts",
        ".github/workflows/ci.yml",
        "src/__tests__/orders.test.ts",
        "Express",
        "Vitest",
        "src-services",
    ] {
        assert!(text.contains(expected), "falta {expected} en el inventario");
    }
    let markers: Vec<String> = inv["markers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| format!("{}:{}", m["path"].as_str().unwrap(), m["line"]))
        .collect();
    assert!(
        markers.contains(&"src/services/tax.ts:1".to_string()),
        "{markers:?}"
    );
    assert!(
        markers.contains(&"src/utils/dates.ts:1".to_string()),
        "{markers:?}"
    );
    // Ningún módulo excede el límite configurado.
    for m in inv["modules"].as_array().unwrap() {
        assert!(m["files"].as_u64().unwrap() <= 5, "{m}");
    }

    sb.run(&["plan"]);
    let tasks = sb.run(&["task", "list"]);
    let contradictions = fs::read_dir(sb.work().join("tasks"))
        .unwrap()
        .flatten()
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .find(|t| t.contains("type: contradictions"))
        .unwrap_or_else(|| panic!("sin tarea de contradicciones: {tasks}"));
    assert!(
        contradictions.contains("- docs/architecture.md"),
        "{contradictions}"
    );
    assert!(
        contradictions.contains("src/services/tax.ts:1"),
        "{contradictions}"
    );
    assert!(fs::read_to_string(sb.root.join("AGENTS.md"))
        .unwrap()
        .contains("npm test"));

    add_adapter(&sb, "fake", FAKE_AGENT);
    let out = sb.run(&["inspect", "--agent", "fake"]);
    assert!(
        !out.contains("falló") && !out.contains("bloqueada"),
        "{out}"
    );
    sb.run(&["check"]);
    let review = sb.run(&["review"]);
    assert!(review.contains("Progreso:  100%"), "{review}");
}

/// Una tarea que falló en una corrida anterior no se reintenta sin
/// `--resume`; el informe debe decir la causa y cómo seguir.
#[cfg(unix)]
#[test]
fn inspect_explains_previous_failure_and_resume_retries_it() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "lazy", "true");
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "lazy"]);

    let out = sb.run(&["inspect", "--agent", "fake"]);
    assert!(
        out.contains("→ T-001 falló en una corrida anterior"),
        "{out}"
    );
    assert!(out.contains("adoc inspect --resume"), "{out}");
    assert!(out.contains("T-001.log"), "{out}");

    let out = sb.run(&["inspect", "--agent", "fake", "--resume"]);
    assert!(out.contains("✓ T-001"), "{out}");
    assert!(!out.contains("bloqueada"), "{out}");
}

/// Un adaptador de Claude sin modo de permisos no puede escribir: `inspect`
/// debe detenerse antes de lanzar el agente.
#[test]
fn inspect_rejects_claude_adapter_without_permissions() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str(
        "\n[agent.adapters.viejo]\ncommand = \"claude\"\nargs = [\"-p\", \"{prompt_file_content}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    sb.run(&["agent", "trust", "viejo", "--yes"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let out = sb
        .cmd()
        .args(["inspect", "--agent", "viejo"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("--permission-mode"), "{err}");
    // No se tocó ninguna tarea.
    assert!(!sb.run(&["task", "list"]).contains("failed"));
}

/// Un proyecto con el estado antiguo `.arqueology-doc/` (nombre anterior) se migra al
/// espacio de trabajo global en el primer comando.
#[test]
fn legacy_state_dir_is_migrated_to_workspace() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    // Simula una versión anterior: el estado vivía dentro del proyecto.
    let work = sb.work();
    fs::rename(&work, sb.root.join(".arqueology-doc")).unwrap();

    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-001"), "{list}");
    assert!(!sb.root.join(".arqueology-doc").exists());
    assert!(sb.work().join("tasks/T-001.md").is_file());
}

#[test]
fn agent_use_switches_agent_and_model() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let toml_path = sb.root.join("archeology-doc.toml");
    // Un proyecto antiguo sin el adaptador devin predefinido.
    let cfg = fs::read_to_string(&toml_path).unwrap();
    // Quita [agent.adapters.devin] y sus subtablas ([agent.adapters.devin.env]).
    let mut kept = vec!["# comentario del usuario"];
    let mut skipping = false;
    for line in cfg.lines() {
        if line.starts_with('[') {
            skipping = line.starts_with("[agent.adapters.devin");
        }
        if !skipping {
            kept.push(line);
        }
    }
    fs::write(&toml_path, kept.join("\n") + "\n").unwrap();

    let out = sb.run(&["agent", "use", "devin"]);
    assert!(out.contains("añadido el adaptador predefinido"), "{out}");
    let cfg = fs::read_to_string(&toml_path).unwrap();
    assert!(cfg.contains("\"smart\""), "{cfg}");
    let out = sb.run(&["agent", "use", "manual"]);
    assert!(out.contains("modo exportación"), "{out}");
    sb.run(&["agent", "use", "claude", "--model", "opus"]);
    let cfg = fs::read_to_string(&toml_path).unwrap();
    assert!(
        cfg.starts_with("# comentario del usuario"),
        "se perdió el formato:\n{cfg}"
    );
    assert!(cfg.contains("default = \"claude\""), "{cfg}");
    assert!(cfg.contains("model = \"opus\""), "{cfg}");
    let list = sb.run(&["agent", "list"]);
    assert!(list.contains("* claude"), "{list}");
    assert!(list.contains("opus"), "{list}");

    sb.run(&["agent", "use", "claude", "--no-model"]);
    assert!(!fs::read_to_string(&toml_path).unwrap().contains("model = "));
    sb.cmd()
        .args(["agent", "use", "inexistente"])
        .assert()
        .failure();
}

/// Tareas exportadas en modo manual: el informe lo explica y, al
/// volver a un agente headless, `--resume` las ejecuta.
#[cfg(unix)]
#[test]
fn manual_tasks_are_explained_and_resumed_by_headless_agent() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["agent", "use", "manual"]);
    sb.run(&["inspect"]);

    let out = sb.run(&["inspect", "--resume"]);
    assert!(out.contains("tareas en modo manual"), "{out}");
    assert!(out.contains("adoc agent use claude"), "{out}");

    let out = sb.run(&["inspect", "--agent", "fake", "--resume"]);
    assert!(out.contains("✓ T-001"), "{out}");
    assert!(!sb.run(&["task", "list"]).contains("manual"));
}

/// Cambiar a un agente headless reabre las tareas exportadas en modo manual:
/// basta `adoc inspect`, sin `--resume`.
#[cfg(unix)]
#[test]
fn agent_use_headless_reopens_manual_tasks() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["agent", "use", "manual"]);
    sb.run(&["inspect"]);
    sb.run(&["task", "done", "T-001", "--force"]);

    let out = sb.run(&["agent", "use", "fake"]);
    assert!(out.contains("vuelven a `pending` para fake"), "{out}");
    let list = sb.run(&["task", "list"]);
    assert!(!list.contains("manual"), "{list}");
    assert!(
        list.contains("T-001    done"),
        "las done no se tocan: {list}"
    );

    let out = sb.run(&["inspect"]);
    assert!(out.contains("✓ T-002"), "{out}");
}

/// `agent use <agente> --reset` reemplaza una definición antigua por la
/// predefinida (p. ej. devin en modo exportación → CLI de Devin).
#[test]
fn agent_use_reset_restores_builtin_adapter() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let toml_path = sb.root.join("archeology-doc.toml");
    let mut cfg = String::new();
    let mut skipping = false;
    for line in fs::read_to_string(&toml_path).unwrap().lines() {
        if line.starts_with('[') {
            skipping = line.starts_with("[agent.adapters.devin");
        }
        if !skipping {
            cfg.push_str(line);
            cfg.push('\n');
        }
    }
    cfg.push_str("\n[agent.adapters.devin]\nargs = []\nmode = \"export\"\n");
    fs::write(&toml_path, cfg).unwrap();

    let out = sb.run(&["agent", "use", "devin", "--reset"]);
    assert!(out.contains("restablecido"), "{out}");
    let cfg = fs::read_to_string(&toml_path).unwrap();
    let devin = &cfg[cfg.find("[agent.adapters.devin]").unwrap()..];
    assert!(devin.contains("command = \"devin\""), "{devin}");
    assert!(devin.contains("mode = \"headless\""), "{devin}");
    sb.cmd()
        .args(["agent", "use", "inventado", "--reset"])
        .assert()
        .failure();
}

/// Agente que termina con una pregunta sin producir nada: la tarea queda en
/// `question`, no se reintenta, y `task answer` la reabre con la respuesta.
#[cfg(unix)]
#[test]
fn agent_question_is_reported_and_answered() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(
        &sb,
        "dudoso",
        "echo 'Revisé el código.'; echo '**¿El tope correcto es 10 % o 20 %?**'",
    );
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let out = sb.run(&["inspect", "--agent", "dudoso"]);
    assert!(out.contains("? T-001"), "{out}");
    assert!(
        out.contains("pregunta  ¿El tope correcto es 10 % o 20 %?"),
        "{out}"
    );
    assert!(out.contains("adoc task answer T-001"), "{out}");
    assert!(sb
        .run(&["task", "list", "--status", "question"])
        .contains("T-001"));
    // Sin respuesta no se reintenta; el informe explica qué falta.
    let out = sb.run(&["inspect", "--agent", "dudoso", "--resume"]);
    assert!(out.contains("→ T-001 espera tu respuesta"), "{out}");

    sb.run(&["task", "answer", "T-001", "Es 10 %, el código está mal"]);
    let task = fs::read_to_string(sb.work().join("tasks/T-001.md")).unwrap();
    assert!(task.contains("# Respuestas"), "{task}");
    assert!(
        task.contains("Pregunta: ¿El tope correcto es 10 % o 20 %?"),
        "{task}"
    );
    assert!(
        task.contains("Respuesta: Es 10 %, el código está mal"),
        "{task}"
    );
    assert!(task.contains("status: pending"), "{task}");

    let out = sb.run(&["task", "run", "T-001", "--agent", "fake"]);
    assert!(out.contains("✓ T-001"), "{out}");
}

/// `task run --interactive` abre el agente con la tarea y valida al salir.
#[cfg(unix)]
#[test]
fn task_run_interactive_validates_output() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    let script = FAKE_AGENT
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    cfg.push_str(&format!(
        "\n[agent.adapters.vivo]\ncommand = \"sh\"\nargs = [\"-c\", \"true\", \"sh\", \"{{prompt_file}}\"]\n\
         interactive_args = [\"-c\", \"{script}\", \"sh\", \"{{prompt_file}}\"]\n"
    ));
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    sb.run(&["agent", "trust", "vivo", "--yes"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let out = sb.run(&["task", "run", "T-001", "--interactive", "--agent", "vivo"]);
    assert!(out.contains("✓ T-001 completada"), "{out}");
    assert!(sb.run(&["task", "list"]).contains("T-001    done"));

    // Agente sin modo interactivo: instrucciones para hacerlo a mano.
    let out = sb.run(&["task", "run", "T-002", "--interactive", "--agent", "manual"]);
    assert!(out.contains("no tiene modo interactivo"), "{out}");
}

#[test]
fn tasks_tell_the_agent_not_to_ask() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let task = fs::read_to_string(sb.work().join("tasks/T-001.md")).unwrap();
    assert!(task.contains("No hagas preguntas"), "{task}");
    assert!(task.contains("open-questions.md"), "{task}");
}

/// Un plan nuevo distinto no se mezcla con el anterior; `--force` lo
/// reemplaza, borra las tareas sobrantes y conserva el estado de las iguales.
#[test]
fn plan_force_replaces_previous_plan() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    let cfg_path = sb.root.join("archeology-doc.toml");
    let cfg = fs::read_to_string(&cfg_path).unwrap();
    fs::write(
        &cfg_path,
        cfg.replace("size = \"auto\"", "size = \"large\""),
    )
    .unwrap();
    let out = sb.run(&["plan"]);
    assert!(out.contains("perfil: grande"), "{out}");
    let large = fs::read_dir(sb.work().join("tasks")).unwrap().count();
    sb.run(&["task", "done", "T-001", "--force"]);

    let cfg = fs::read_to_string(&cfg_path).unwrap();
    fs::write(
        &cfg_path,
        cfg.replace("size = \"large\"", "size = \"small\""),
    )
    .unwrap();
    let out = sb.run(&["plan"]);
    assert!(out.contains("no se mezclan"), "{out}");

    let out = sb.run(&["plan", "--force"]);
    let small = fs::read_dir(sb.work().join("tasks")).unwrap().count();
    assert!(small < large, "{small} >= {large}");
    assert!(out.contains("eliminadas"), "{out}");
    // T-001 (overview) cambió de salidas en el perfil pequeño (incluye glosario).
    assert!(sb.run(&["task", "list"]).contains("T-001"));
}

/// `adoc status` (JSON para la extensión de VS Code) y `task list --format json`.
#[test]
fn status_json_describes_project_for_integrations() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    fs::write(
        sb.root.join("knowledge/findings/x.md"),
        "---\nid: find-x\ntype: finding\ntitle: X\nkind: risk\nseverity: high\nstate: open\n\
         sources:\n  - src/main.py:1\n---\n",
    )
    .unwrap();

    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    assert_eq!(st["agent"]["active"], "manual");
    assert_eq!(st["profile"], "small");
    assert!(st["tasks"][0]["file"].as_str().unwrap().starts_with('/'));
    assert_eq!(st["findings"][0]["sources"][0], "src/main.py:1");
    assert!(st["agent"]["available"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "devin"));

    let tasks: serde_json::Value =
        serde_json::from_str(&sb.run(&["task", "list", "--format", "json", "--status", "pending"]))
            .unwrap();
    assert!(tasks
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["status"] == "pending"));
}

/// Reporte HTML y dashboard del sitio: estado, hallazgos, preguntas
/// pendientes (agente y equipo) y FAQ; el texto se escapa.
#[cfg(unix)]
#[test]
fn report_and_site_dashboard_show_findings_and_questions() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "dudoso", "echo '¿Cuál es el tope?'");
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "dudoso"]);
    fs::write(
        sb.root.join("knowledge/findings/x.md"),
        "---\nid: find-x\ntype: finding\ntitle: \"Tope <script>alert(1)</script>\"\nkind: risk\n\
         severity: high\nstate: open\nsources:\n  - src/main.py:1\n---\n",
    )
    .unwrap();
    let mut open_q =
        fs::read_to_string(sb.root.join("knowledge/memory/open-questions.md")).unwrap();
    open_q.push_str("- 2026-10-09 · ¿Quién aprueba los reembolsos? · fuente: src/main.py:1\n");
    fs::write(sb.root.join("knowledge/memory/open-questions.md"), open_q).unwrap();
    fs::create_dir_all(sb.root.join("knowledge/faq")).unwrap();
    fs::write(
        sb.root.join("knowledge/faq/como-arranca.md"),
        "---\nid: faq-como-arranca\ntype: faq\ntitle: \"¿Cómo arranca?\"\nsources: [src/main.py:1]\n---\n\n# ¿Cómo arranca?\n\nCon main().\n",
    )
    .unwrap();

    let out = sb.root.join("r.html");
    sb.run(&["report", "--out", out.to_str().unwrap(), "--no-open"]);
    let html = fs::read_to_string(&out).unwrap();
    assert!(html.contains("&lt;script&gt;"), "sin escapar");
    assert!(!html.contains("<script>alert"), "XSS");
    assert!(html.contains("¿Cuál es el tope?"), "pregunta del agente");
    assert!(
        html.contains("¿Quién aprueba los reembolsos?"),
        "pregunta del equipo"
    );
    assert!(html.contains("Con main()."), "FAQ");

    fs::write(
        sb.root.join("docs/index.md"),
        "---\nlayout: home\n---\n\nTexto del equipo.\n",
    )
    .unwrap();
    sb.run(&["site", "sync"]);
    let index = fs::read_to_string(sb.root.join("docs/index.md")).unwrap();
    assert!(
        index.contains("Texto del equipo.") && index.contains("begin dashboard"),
        "{index}"
    );
    let preguntas = fs::read_to_string(sb.root.join("docs/preguntas.md")).unwrap();
    assert!(preguntas.contains("adoc task answer T-001"), "{preguntas}");
    let sidebar =
        fs::read_to_string(sb.root.join("docs/.vitepress/sidebar.generated.json")).unwrap();
    assert!(sidebar.contains("/hallazgos"), "{sidebar}");
}

/// `ask` con un agente que responde por pantalla: la respuesta queda en la FAQ.
#[cfg(unix)]
#[test]
fn ask_saves_answer_in_faq() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "sabio", "echo 'Arranca en src/main.py:1 con main().'");
    let out = sb.run(&["ask", "¿Cómo arranca la app?", "--agent", "sabio"]);
    assert!(out.contains("Arranca en src/main.py:1"), "{out}");
    let faq = fs::read_dir(sb.root.join("knowledge/faq")).unwrap().count();
    assert!(faq >= 1);
    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    assert_eq!(
        st["questions"]["faq"][0]["question"],
        "¿Cómo arranca la app?"
    );
}

/// La pregunta puede ir sin comillas y con flags en cualquier posición.
#[cfg(unix)]
#[test]
fn ask_accepts_unquoted_words() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "sabio", "echo 'Sí, por día completo.'");
    let out = sb.run(&[
        "ask",
        "las",
        "reservas",
        "son",
        "por",
        "dia",
        "completo?",
        "--agent",
        "sabio",
    ]);
    assert!(out.contains("Sí, por día completo."), "{out}");
    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    assert_eq!(
        st["questions"]["faq"][0]["question"],
        "las reservas son por dia completo?"
    );
    // Sin pregunta y sin terminal: error claro, no una pregunta vacía al agente.
    sb.cmd()
        .args(["ask", "--agent", "sabio"])
        .assert()
        .failure();
}

/// `adoc graph`: notas, wikilinks y evidencias para la vista de grafo.
#[test]
fn graph_json_has_notes_links_and_code() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    fs::write(
        sb.root.join("knowledge/modules/src.md"),
        "---\nid: mod-src\ntype: module\ntitle: src\nsources: [src/main.py:1]\n---\nVer [[00-overview]].\n",
    )
    .unwrap();
    sb.run(&["scan"]);
    let g: serde_json::Value = serde_json::from_str(&sb.run(&["graph", "--json"])).unwrap();
    let edges = g["edges"].as_array().unwrap();
    assert!(edges
        .iter()
        .any(|e| e["source"] == "modules/src" && e["target"] == "00-overview"));
    assert!(edges
        .iter()
        .any(|e| e["target"] == "code:src/main.py" && e["kind"] == "evidence"));
    assert!(g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "code" && n["line"] == 1));
}

/// Vault con un título hostil, un enlace y una nota huérfana (tests del grafo).
fn graph_fixture(sb: &Sandbox) {
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    fs::write(
        sb.root.join("knowledge/modules/src.md"),
        "---\nid: mod-src\ntype: module\ntitle: \"Src </script><script>alert(1)</script>\"\n\
         sources: [src/main.py:1]\n---\nVer [[00-overview]].\n",
    )
    .unwrap();
    fs::write(
        sb.root.join("knowledge/modules/lonely.md"),
        "---\nid: mod-lonely\ntype: module\ntitle: Solitaria\n---\nSin enlaces.\n",
    )
    .unwrap();
}

/// `adoc graph --format html --out`: HTML autocontenido con los datos
/// escapados, CSP estricta y sin recursos externos.
#[test]
fn graph_html_is_self_contained_and_escaped() {
    let sb = Sandbox::new();
    graph_fixture(&sb);
    let out = sb.root.join("g.html");
    let stdout = sb.run(&["graph", "--format", "html", "--out", out.to_str().unwrap()]);
    assert!(stdout.contains("g.html"), "{stdout}");
    let html = fs::read_to_string(&out).unwrap();
    assert!(html.contains("Content-Security-Policy"), "sin CSP");
    assert!(html.contains("script-src 'nonce-"), "CSP sin nonce");
    assert!(html.contains("AdocGraph"), "sin renderizador");
    assert!(html.contains("modules/src"), "sin datos");
    assert!(html.contains("code:src/main.py"), "sin nodos de código");
    // El título hostil no cierra el bloque de datos.
    assert!(!html.contains("<script>alert"), "XSS");
    assert!(html.contains("\\u003c/script\\u003e"), "título sin escapar");
    // Sin recursos externos.
    assert!(
        !html.contains("http://") && !html.contains("https://"),
        "URL externa"
    );
    assert!(!html.contains(" src="), "script externo");
    // Textos en el idioma de la interfaz (es en estos tests).
    assert!(html.contains("Grafo del conocimiento"), "idioma");
    // Por defecto, en el espacio de trabajo; `--open` con otro formato es un error.
    sb.run(&["graph", "--format", "html"]);
    assert!(sb.work().join("graph.html").is_file());
    sb.cmd()
        .args(["graph", "--format", "json", "--open"])
        .assert()
        .failure();
}

/// `review` muestra la sección del grafo (texto en en/es, markdown) y la
/// añade al JSON como clave nueva.
#[test]
fn review_shows_graph_section() {
    let sb = Sandbox::new();
    graph_fixture(&sb);
    let es = sb.run(&["review"]);
    assert!(es.contains("== Grafo =="), "{es}");
    assert!(es.contains("Notas huérfanas ("), "{es}");
    assert!(es.contains("modules/lonely"), "{es}");
    assert!(es.contains("Más conectadas: Src"), "{es}");
    assert!(es.contains("adoc graph --open"), "{es}");
    let out = sb
        .cmd()
        .env("ADOC_LANG", "en")
        .args(["review"])
        .assert()
        .success();
    let en = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(en.contains("== Graph =="), "{en}");
    assert!(
        en.contains("Orphan notes (") && en.contains("modules/lonely"),
        "{en}"
    );
    assert!(en.contains("Most connected:"), "{en}");
    let md = sb.run(&["review", "--format", "markdown"]);
    assert!(md.contains("## Grafo"), "{md}");
    let json: serde_json::Value =
        serde_json::from_str(&sb.run(&["review", "--format", "json"])).unwrap();
    assert!(json["graph"]["orphans"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["id"] == "modules/lonely"));
    assert_eq!(json["graph"]["top_connected"][0]["id"], "modules/src");
    assert!(json["graph"]["edges_by_kind"]["evidence"].as_u64().unwrap() >= 1);
    // Las claves existentes no cambian.
    assert!(json["progress"].is_number() && json["tasks"].is_object());
}

/// `site sync` publica la página del grafo, sus recursos y el enlace en el
/// sidebar; `report` incrusta el grafo (HTML) o su resumen (markdown).
#[test]
fn site_and_report_include_graph() {
    let sb = Sandbox::new();
    graph_fixture(&sb);
    sb.run(&["site", "sync"]);
    let docs = sb.root.join("docs");
    let page = fs::read_to_string(docs.join("grafo.md")).unwrap();
    assert!(page.contains("id=\"adoc-graph\""), "{page}");
    assert!(page.contains("adoc-graph/"), "{page}");
    assert!(page.contains("Notas huérfanas"), "{page}");
    assert!(!page.contains("<script>alert"), "{page}");
    let assets = docs.join("public/adoc-graph");
    assert!(assets.join(".generated-by-archeology-doc").is_file());
    let js = fs::read_to_string(assets.join("graph.js")).unwrap();
    assert!(js.contains("class Layout"));
    assert!(assets.join("graph.css").is_file());
    let data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(assets.join("graph.json")).unwrap()).unwrap();
    let src = data["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "modules/src")
        .unwrap();
    assert_eq!(src["link"], "/wiki/modules/src");
    assert!(data["strings"]["search"].is_string());
    // El sitio puede publicarse: sin rutas absolutas locales.
    let raw = fs::read_to_string(assets.join("graph.json")).unwrap();
    assert!(!raw.contains(sb.root.to_str().unwrap()), "ruta local");
    let sidebar = fs::read_to_string(docs.join(".vitepress/sidebar.generated.json")).unwrap();
    assert!(sidebar.contains("/grafo"), "{sidebar}");
    let index = fs::read_to_string(docs.join("index.md")).unwrap();
    assert!(index.contains("(/grafo)"), "{index}");
    // Idempotente; una carpeta de recursos del usuario no se pisa.
    sb.run(&["site", "sync"]);
    fs::remove_file(assets.join(".generated-by-archeology-doc")).unwrap();
    sb.cmd().args(["site", "sync"]).assert().failure();
    fs::remove_dir_all(&assets).unwrap();
    // Los recursos del grafo no se inventarían como código.
    sb.run(&["site", "sync"]);
    let inv = sb.run(&["scan", "--json"]);
    assert!(!inv.contains("adoc-graph"), "{inv}");

    let out = sb.root.join("r.html");
    sb.run(&["report", "--out", out.to_str().unwrap(), "--no-open"]);
    let html = fs::read_to_string(&out).unwrap();
    assert!(html.contains("id=\"grafo\""), "sin sección");
    assert!(html.contains("AdocGraph"), "sin renderizador");
    assert!(html.contains("Content-Security-Policy"), "sin CSP");
    assert!(!html.contains("<script>alert"), "XSS");
    assert!(html.contains("Solitaria"), "huérfana");
    let md = sb.root.join("r.md");
    sb.run(&[
        "report",
        "--format",
        "markdown",
        "--out",
        md.to_str().unwrap(),
        "--no-open",
    ]);
    let md = fs::read_to_string(&md).unwrap();
    assert!(md.contains("## Grafo del conocimiento"), "{md}");
    assert!(md.contains("Notas más conectadas"), "{md}");
}

/// Un `archeology-doc.toml` hostil no ejecuta nada sin aprobación: ni con
/// `doctor`/`agent list` (que antes lanzaban `<cmd> --version`) ni con `inspect`.
#[cfg(unix)]
#[test]
fn repo_defined_adapter_never_runs_without_trust() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg = cfg.replace("default = \"manual\"", "default = \"evil\"");
    cfg.push_str(
        "\n[agent.adapters.evil]\ncommand = \"sh\"\nargs = [\"-c\", \"touch PWNED\", \"sh\", \"{prompt_file}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();

    let _ = sb.cmd().arg("doctor").output().unwrap();
    let out = sb.run(&["agent", "list"]);
    assert!(out.contains("sin aprobar"), "{out}");
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let out = sb.cmd().arg("inspect").assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("adoc agent trust evil"), "{err}");
    assert!(!sb.root.join("PWNED").exists(), "se ejecutó el adaptador");

    // Aprobado, se ejecuta; si luego cambia en el repositorio, deja de estarlo.
    sb.run(&["agent", "trust", "evil", "--yes"]);
    assert!(!sb.run(&["agent", "list"]).contains("sin aprobar"));
    let cfg = fs::read_to_string(sb.root.join("archeology-doc.toml"))
        .unwrap()
        .replace("touch PWNED", "touch PWNED2");
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    assert!(sb.run(&["agent", "list"]).contains("sin aprobar"));
}

/// Un `project.id` con `..` no puede sacar el espacio de trabajo de
/// `projects/` ni migrar archivos del repositorio fuera de él.
#[test]
fn hostile_project_id_is_rejected() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    let id_line = cfg.lines().find(|l| l.starts_with("id = ")).unwrap();
    let cfg = cfg.replace(id_line, "id = \"../..\"");
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    fs::create_dir_all(sb.root.join(".archeology-doc")).unwrap();
    fs::write(sb.root.join(".archeology-doc/.zshenv"), "malicioso").unwrap();

    let out = sb.cmd().arg("status").assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("project.id inválido"), "{err}");
    assert!(sb.root.join(".archeology-doc/.zshenv").is_file());
    assert!(!sb.config.join(".zshenv").exists());
}

/// Rutas de salida fuera del proyecto se rechazan antes de escribir nada.
#[test]
fn output_paths_outside_project_are_rejected() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let cfg = fs::read_to_string(sb.root.join("archeology-doc.toml"))
        .unwrap()
        .replace("docs = \"docs\"", "docs = \"../fuera\"");
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    let out = sb.cmd().args(["site", "sync"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("paths.docs"), "{err}");
    assert!(!sb.root.parent().unwrap().join("fuera").exists());
}

/// `docs/wiki` escrito a mano (docs/ existía antes de `init`) no se borra.
#[test]
fn site_sync_keeps_user_owned_wiki() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    fs::create_dir_all(sb.root.join("docs/wiki")).unwrap();
    fs::write(sb.root.join("docs/wiki/manual.md"), "# Mío\n").unwrap();
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let out = sb.cmd().args(["site", "sync"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("no lo generó archeology-doc"), "{err}");
    assert!(sb.root.join("docs/wiki/manual.md").is_file());
}

/// Un proyecto inicializado con el nombre anterior (arqueology-doc) se migra:
/// archivo de configuración y marcadores de AGENTS.md, sin duplicar regiones.
#[test]
fn project_with_previous_name_is_migrated() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    fs::rename(
        sb.root.join("archeology-doc.toml"),
        sb.root.join("arqueology-doc.toml"),
    )
    .unwrap();
    let agents = fs::read_to_string(sb.root.join("AGENTS.md"))
        .unwrap()
        .replace("archeology-doc:", "arqueology-doc:");
    fs::write(sb.root.join("AGENTS.md"), agents).unwrap();

    sb.run(&["scan"]);
    assert!(sb.root.join("archeology-doc.toml").is_file());
    assert!(!sb.root.join("arqueology-doc.toml").exists());
    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    assert!(!agents.contains("arqueology-doc:"), "{agents}");
    assert_eq!(agents.matches("archeology-doc:begin context").count(), 1);
}

/// Quita `[agent.adapters.<name>]` (y sus subtablas) del TOML del proyecto.
fn strip_adapter(sb: &Sandbox, name: &str) -> String {
    let header = format!("[agent.adapters.{name}");
    let mut cfg = String::new();
    let mut skipping = false;
    for line in fs::read_to_string(sb.root.join("archeology-doc.toml"))
        .unwrap()
        .lines()
    {
        if line.starts_with('[') {
            skipping = line == format!("{header}]") || line.starts_with(&format!("{header}."));
        }
        if !skipping {
            cfg.push_str(line);
            cfg.push('\n');
        }
    }
    cfg
}

/// `[agent.adapters.claude]` con la definición peligrosa de `amp` no pasa por
/// predefinido: requiere aprobación como cualquier adaptador del repositorio.
#[test]
fn spoofed_builtin_name_requires_trust() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = strip_adapter(&sb, "claude");
    cfg.push_str(
        "\n[agent.adapters.claude]\ncommand = \"amp\"\n\
         args = [\"--dangerously-allow-all\", \"-x\", \"{prompt_file_content}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    let list = sb.run(&["agent", "list"]);
    let row = list.lines().find(|l| l.contains(" claude ")).unwrap();
    assert!(row.contains("sin aprobar"), "{list}");
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let out = sb
        .cmd()
        .args(["inspect", "--agent", "claude"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("adoc agent trust claude"), "{err}");
    // `agent trust` muestra la definición y pide confirmación (no "predefinido").
    let out = sb
        .cmd()
        .args(["agent", "trust", "claude"])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(!stdout.contains("no necesita aprobación"), "{stdout}");
    assert!(stdout.contains("--dangerously-allow-all"), "{stdout}");
}

/// `agent trust` escapa los caracteres de control (no pueden ocultar lo que
/// se aprueba) y muestra el modelo; cambiar un modelo usado en `{model}`
/// exige volver a aprobar.
#[test]
fn agent_trust_escapes_control_chars_and_shows_model() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str(
        "\n[agent.adapters.raro]\ncommand = \"sh\\u001b[2K\"\n\
         args = [\"-c\", \"true\", \"--modelo={model}\", \"{prompt_file}\"]\nmodel = \"opus\"\n\
         [agent.adapters.raro.env]\nCLAVE = \"v\\r\\u001b[1Aoculto\"\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), &cfg).unwrap();
    let out = sb.run(&["agent", "trust", "raro", "--yes"]);
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(!out.contains('\r'), "{out:?}");
    assert!(out.contains("\\u{1b}"), "{out}");
    assert!(out.contains("model   = \"opus\""), "{out}");
    assert!(!sb.run(&["agent", "list"]).contains("sin aprobar"));
    // El modelo entra en `{model}`: cambiarlo en el repositorio exige aprobar de nuevo.
    fs::write(
        sb.root.join("archeology-doc.toml"),
        cfg.replace("model = \"opus\"", "model = \"otro\""),
    )
    .unwrap();
    assert!(sb.run(&["agent", "list"]).contains("sin aprobar"));
}

/// `task run` con un agente sin aprobar no toca la tarea: una `done` sigue `done`.
/// `task answer` solo responde tareas en `question`.
#[test]
fn task_run_with_untrusted_agent_keeps_done_task() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str(
        "\n[agent.adapters.ajeno]\ncommand = \"sh\"\nargs = [\"-c\", \"touch PWNED\", \"sh\", \"{prompt_file}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["task", "done", "T-001", "--force"]);

    let out = sb
        .cmd()
        .args(["task", "run", "T-001", "--agent", "ajeno"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("adoc agent trust ajeno"), "{err}");
    assert!(sb.run(&["task", "list"]).contains("T-001    done"));
    assert!(!sb.root.join("PWNED").exists());
    // Agente inexistente: tampoco se toca.
    sb.cmd()
        .args(["task", "run", "T-001", "--agent", "inventado"])
        .assert()
        .failure();
    assert!(sb.run(&["task", "list"]).contains("T-001    done"));

    // `task answer` sobre una tarea que no pregunta nada: error, sigue done.
    let out = sb
        .cmd()
        .args(["task", "answer", "T-001", "hola"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("question"), "{err}");
    assert!(sb.run(&["task", "list"]).contains("T-001    done"));
}

/// `agent use` con un agente headless sin aprobar no reabre las tareas manuales.
#[test]
fn agent_use_untrusted_headless_keeps_manual_tasks() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str(
        "\n[agent.adapters.nuevo]\ncommand = \"sh\"\nargs = [\"-c\", \"true\", \"sh\", \"{prompt_file}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect"]);
    let out = sb.run(&["agent", "use", "nuevo"]);
    assert!(out.contains("no se reabren"), "{out}");
    assert!(sb.run(&["task", "list"]).contains("manual"));
}

/// `agent list`/`agent test` no esconden una config inválida tras los predefinidos.
#[test]
fn agent_list_reports_invalid_config() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str("\n[agent.adapters.mal]\ncommand = \"x\"\nmodel = \"--yolo\"\n");
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    let out = sb.cmd().args(["agent", "list"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("model"), "{err}");
    sb.cmd().args(["agent", "test"]).assert().failure();
    // Sin proyecto, se listan los predefinidos.
    let tmp = tempfile::tempdir().unwrap();
    adoc_cmd("archeology-doc", &sb.config)
        .current_dir(tmp.path())
        .args(["agent", "list"])
        .assert()
        .success();
}

/// `ask` con un agente que falla no deja una nota a medias en la FAQ.
#[cfg(unix)]
#[test]
fn ask_discards_note_when_agent_fails() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(
        &sb,
        "roto",
        "f=$(grep -m1 '^- knowledge/faq/' \"$1\" | cut -c3-); mkdir -p \"$(dirname \"$f\")\"; \
         printf -- '---\\nid: x\\n---\\nmitad' > \"$f\"; exit 1",
    );
    let out = sb
        .cmd()
        .args(["ask", "¿Cómo arranca?", "--agent", "roto"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("exit code 1"), "{err}");
    let faq: Vec<_> = fs::read_dir(sb.root.join("knowledge/faq"))
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        faq.iter()
            .all(|f| !f.to_string_lossy().ends_with(".md")
                || f.to_string_lossy().starts_with("index")),
        "{faq:?}"
    );
    let partial = fs::read_dir(sb.work().join("asks"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".partial.md"));
    assert!(partial, "{err}");
}

/// Ctrl+C durante `inspect`: se mata al agente (aunque esté en otro grupo de
/// procesos), la tarea vuelve a `pending`, queda el log y `adoc` sale con 130.
#[cfg(unix)]
#[test]
fn ctrl_c_kills_agent_and_leaves_task_resumable() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "lento", "echo $$ > agent.pid; exec sleep 60");
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("archeology-doc"))
        .env("ARCHEOLOGY_DOC_CONFIG_DIR", &sb.config)
        .env("ADOC_LANG", "es")
        .current_dir(&sb.root)
        .args(["inspect", "--agent", "lento"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid_file = sb.root.join("agent.pid");
    let start = std::time::Instant::now();
    let agent_pid = loop {
        if let Some(pid) = fs::read_to_string(&pid_file)
            .ok()
            .filter(|t| t.ends_with('\n'))
        {
            break pid.trim().to_string();
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(30));
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let status = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let code = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s.code();
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "adoc no terminó"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(code, Some(130));
    let alive = std::process::Command::new("kill")
        .args(["-0", &agent_pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "el agente quedó huérfano");
    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-001    pending"), "{list}");
    assert!(!list.contains("running"), "{list}");
    let run = fs::read_dir(sb.work().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let log = fs::read_to_string(run.path().join("run.log")).unwrap();
    assert!(log.contains("interrumpida"), "{log}");
}

/// Un almacén de aprobaciones dañado no se reemplaza en silencio: aprobar
/// falla y el archivo queda intacto (no se pierden las demás aprobaciones).
#[test]
fn corrupt_trust_store_is_not_wiped() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let mut cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    cfg.push_str(
        "\n[agent.adapters.propio]\ncommand = \"sh\"\nargs = [\"-c\", \"true\", \"sh\", \"{prompt_file}\"]\n",
    );
    fs::write(sb.root.join("archeology-doc.toml"), cfg).unwrap();
    let store = sb.config.join("trusted-adapters.toml");
    fs::write(&store, "[[adapter]\nroto").unwrap();
    let out = sb
        .cmd()
        .args(["agent", "trust", "propio", "--yes"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("dañado"), "{err}");
    assert_eq!(fs::read_to_string(&store).unwrap(), "[[adapter]\nroto");
    // Falla cerrado: nada cuenta como aprobado.
    assert!(sb.run(&["agent", "list"]).contains("sin aprobar"));
    // `agent untrust` en un almacén inexistente no falla.
    fs::remove_file(&store).unwrap();
    sb.run(&["agent", "untrust", "propio"]);
}

// --- Idioma (i18n) -----------------------------------------------------------

fn json_out(cmd: &mut Command) -> serde_json::Value {
    let out = cmd.assert().success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

#[test]
fn ui_language_defaults_to_english() {
    let sb = Sandbox::new();
    let v = json_out(
        sb.cmd_without_lang()
            .args(["config", "get", "language", "--json"]),
    );
    assert_eq!(v["value"], "en", "{v}");
    assert_eq!(v["source"], "default", "{v}");
    // Mensajes de error en inglés.
    let out = sb.cmd_without_lang().args(["scan"]).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("no archeology-doc project found"), "{err}");
    // El locale del sistema cuenta si no hay nada más.
    let v = json_out(
        sb.cmd_without_lang()
            .env("LANG", "es_CR.UTF-8")
            .args(["config", "get", "language", "--json"]),
    );
    assert_eq!(
        (v["value"].as_str(), v["source"].as_str()),
        (Some("es"), Some("locale"))
    );
}

#[test]
fn lang_flag_beats_env_and_invalid_values_are_skipped() {
    let sb = Sandbox::new();
    let v = json_out(
        sb.cmd()
            .env("ADOC_LANG", "en")
            .args(["--lang", "es", "config", "get", "language", "--json"]),
    );
    assert_eq!(
        (v["value"].as_str(), v["source"].as_str()),
        (Some("es"), Some("flag"))
    );
    let v = json_out(
        sb.cmd()
            .env("ADOC_LANG", "en")
            .args(["config", "get", "language", "--json"]),
    );
    assert_eq!(
        (v["value"].as_str(), v["source"].as_str()),
        (Some("en"), Some("env"))
    );
    // Valor inválido: aviso y siguiente fuente.
    let out = sb
        .cmd()
        .args(["config", "get", "language", "--lang=fr"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        stdout.starts_with("es (origen: la variable de entorno ADOC_LANG)"),
        "{stdout}"
    );
    assert!(stderr.contains("'fr'"), "{stderr}");
}

#[test]
fn config_set_get_unset_language() {
    let sb = Sandbox::new();
    fs::create_dir_all(&sb.config).unwrap();
    let cfg = sb.config.join("config.toml");
    fs::write(&cfg, "default_agent = \"claude\"\n").unwrap();

    let out = sb.run(&["config", "set", "language", "en"]);
    assert!(out.contains("language = en"), "{out}");
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("language = \"en\""), "{text}");
    assert!(text.contains("default_agent = \"claude\""), "{text}");
    let v = json_out(
        sb.cmd_without_lang()
            .env("LANG", "es_ES.UTF-8")
            .args(["config", "get", "language", "--json"]),
    );
    assert_eq!(
        (v["value"].as_str(), v["source"].as_str()),
        (Some("en"), Some("config"))
    );
    // Texto legible en el idioma resuelto (inglés, por la configuración).
    let out = sb
        .cmd_without_lang()
        .args(["config", "get", "language"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.starts_with("en (source: the user configuration"),
        "{stdout}"
    );

    sb.cmd()
        .args(["config", "set", "language", "fr"])
        .assert()
        .failure();
    sb.run(&["config", "unset", "language"]);
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(!text.contains("language"), "{text}");
    assert!(text.contains("default_agent"), "{text}");
    let v = json_out(
        sb.cmd_without_lang()
            .args(["config", "get", "language", "--json"]),
    );
    assert_eq!(v["source"], "default", "{v}");
}

#[test]
fn init_writes_project_language() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    // Sin --lang: el idioma de la interfaz resuelto (ADOC_LANG=es).
    sb.run(&["init", "--yes"]);
    let toml = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    assert!(toml.contains("language = \"es\""), "{toml}");
    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("## Contexto para agentes (archeology-doc)"),
        "{agents}"
    );

    // --lang inválido en init: error (no solo aviso).
    let other = Sandbox::new();
    other
        .cmd()
        .args(["init", "--yes", "--lang", "fr"])
        .assert()
        .failure();
    assert!(!other.root.join("archeology-doc.toml").exists());
}

#[test]
fn init_with_lang_en_generates_english_content() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--lang", "en"]);
    let root = &sb.root;
    let toml = fs::read_to_string(root.join("archeology-doc.toml")).unwrap();
    assert!(toml.contains("language = \"en\""), "{toml}");
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("## Agent context (archeology-doc)"),
        "{agents}"
    );
    assert!(agents.contains("Write the notes in English."), "{agents}");
    let feature = fs::read_to_string(root.join("knowledge/_templates/feature.md")).unwrap();
    assert!(feature.contains("## Main flow"), "{feature}");
    let facts = fs::read_to_string(root.join("knowledge/memory/facts.md")).unwrap();
    assert!(facts.contains("# Confirmed facts"), "{facts}");
    let llms = fs::read_to_string(root.join("llms.txt")).unwrap();
    assert!(llms.contains("Index for LLMs"), "{llms}");
    let vp = fs::read_to_string(root.join("docs/.vitepress/config.mts")).unwrap();
    assert!(vp.contains("lang: 'en-US'"), "{vp}");

    // La interfaz sigue en español (ADOC_LANG=es); el contenido, en inglés.
    let out = sb.cmd().args(["scan"]).assert().success();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(!err.contains("Initializing"), "{err}");
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("## Project (archeology-doc inventory)"),
        "{agents}"
    );
    sb.run(&["plan"]);
    let task = fs::read_to_string(sb.work().join("tasks/T-001.md")).unwrap();
    assert!(task.contains("# Objective\n"), "{task}");
    assert!(task.contains("Write the notes in English."), "{task}");
}

#[test]
fn config_without_language_generates_spanish_content() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--lang", "en"]);
    // Configuración anterior a la internacionalización: sin el campo.
    let path = sb.root.join("archeology-doc.toml");
    let toml = fs::read_to_string(&path).unwrap();
    let without: String = toml
        .lines()
        .filter(|l| !l.trim_start().starts_with("language"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&path, without).unwrap();
    fs::remove_file(sb.root.join("AGENTS.md")).unwrap();

    // Interfaz en inglés, contenido en español.
    sb.cmd()
        .env("ADOC_LANG", "en")
        .args(["scan"])
        .assert()
        .success();
    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("## Contexto para agentes (archeology-doc)"),
        "{agents}"
    );
    assert!(
        agents.contains("## Proyecto (inventario de archeology-doc)"),
        "{agents}"
    );
    sb.cmd()
        .env("ADOC_LANG", "en")
        .args(["plan"])
        .assert()
        .success();
    let task = fs::read_to_string(sb.work().join("tasks/T-001.md")).unwrap();
    assert!(task.contains("# Objetivo\n"), "{task}");
}

// --- i18n fase 2: comandos de trabajo ---

/// Ejecuta `args` con la interfaz en inglés y devuelve (stdout, stderr).
fn run_en(sb: &Sandbox, args: &[&str], ok: bool) -> (String, String) {
    let mut cmd = sb.cmd();
    cmd.env("ADOC_LANG", "en").args(args);
    let out = if ok {
        cmd.assert().success()
    } else {
        cmd.assert().failure()
    };
    let o = out.get_output();
    (
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

#[test]
fn work_commands_speak_english() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);

    // Sin inventario: plan falla con el mensaje en inglés.
    let (_, err) = run_en(&sb, &["plan"], false);
    assert!(
        err.contains("no inventory found; run `adoc scan` first"),
        "{err}"
    );

    sb.run(&["scan"]);
    let (out, _) = run_en(&sb, &["plan"], true);
    assert!(out.contains("profile: "), "{out}");
    assert!(
        out.contains("Next step: `adoc inspect` or `adoc task list`."),
        "{out}"
    );

    let (out, _) = run_en(&sb, &["task", "list"], true);
    assert!(out.contains("task") && out.contains("status"), "{out}");
    let (_, err) = run_en(&sb, &["task", "show", "T-999"], false);
    assert!(err.contains("task T-999 does not exist"), "{err}");
    let (_, err) = run_en(&sb, &["task", "show", "bad id"], false);
    assert!(err.contains("invalid task id"), "{err}");

    let (out, _) = run_en(&sb, &["inspect", "--dry-run"], true);
    assert!(out.contains("tasks would run:"), "{out}");

    let (out, _) = run_en(&sb, &["check"], true);
    assert!(out.contains(" · 0 errors"), "{out}");

    let (out, _) = run_en(&sb, &["sync", "--dry-run"], true);
    assert!(out.contains("DRY-RUN"), "{out}");
    assert!(out.contains("No changes"), "{out}");

    // Cambio de agente: mensajes de `agent use` en inglés.
    let (out, _) = run_en(&sb, &["agent", "use", "manual"], true);
    assert!(out.contains("active agent: manual"), "{out}");
    assert!(out.contains("export mode"), "{out}");
}

// --- i18n fase 2: sitio y reportes ---

/// Ejecuta con la interfaz en inglés y devuelve stdout (debe terminar bien).
fn site_run_en(sb: &Sandbox, args: &[&str]) -> String {
    let out = sb
        .cmd()
        .env("ADOC_LANG", "en")
        .args(args)
        .assert()
        .success();
    String::from_utf8_lossy(&out.get_output().stdout).to_string()
}

#[test]
fn site_and_report_commands_speak_english() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--lang", "en"]);

    // scan: resumen en inglés y región del inventario en el idioma del proyecto.
    let out = site_run_en(&sb, &["scan"]);
    assert!(out.contains("Next step: `adoc plan`"), "{out}");
    let overview = fs::read_to_string(sb.root.join("knowledge/00-overview.md")).unwrap();
    assert!(overview.contains("## Summary"), "{overview}");
    assert!(overview.contains("- Entry points:"), "{overview}");

    sb.run(&["plan"]);
    let out = site_run_en(&sb, &["review"]);
    assert!(out.contains("Progress:  0%"), "{out}");
    let out = site_run_en(&sb, &["status", "--format", "text"]);
    assert!(out.contains(" tasks · "), "{out}");

    // doctor: el JSON conserva claves y nombres; el detalle se traduce.
    let out = sb
        .cmd()
        .env("ADOC_LANG", "en")
        .args(["doctor", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let checks = json["checks"].as_array().unwrap();
    let adoc = checks.iter().find(|c| c["name"] == "adoc").unwrap();
    assert!(
        adoc["detail"].as_str().unwrap().starts_with("version "),
        "{adoc}"
    );
    assert!(checks.iter().any(|c| c["name"] == "proyecto"), "{json}");

    // Reporte HTML: contenido en el idioma del proyecto (en), aunque la
    // interfaz esté en español.
    let html_path = sb.root.join("report.html");
    let out = sb.run(&[
        "report",
        "--html",
        "--no-open",
        "--out",
        html_path.to_str().unwrap(),
    ]);
    assert!(out.contains("reporte: "), "{out}");
    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("<html lang=\"en\">"), "{html}");
    assert!(html.contains("Open findings"), "{html}");
    assert!(html.contains("Generated by archeology-doc"), "{html}");

    // Páginas del dashboard del sitio, también en inglés.
    let out = sb
        .cmd()
        .env("ADOC_LANG", "en")
        .args(["site", "sync"])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("pages in the sidebar"), "{err}");
    let estado = fs::read_to_string(sb.root.join("docs/estado.md")).unwrap();
    assert!(estado.contains("- **Plan progress:**"), "{estado}");
    let preguntas = fs::read_to_string(sb.root.join("docs/preguntas.md")).unwrap();
    assert!(
        preguntas.contains("## Frequently asked questions (FAQ)"),
        "{preguntas}"
    );
}

// --- i18n fase 2: ayuda y crates ---

/// `--help` sigue el idioma de la interfaz (ADOC_LANG o --lang), también en
/// subcomandos y en el subcomando `help` de clap.
#[test]
fn help_follows_the_interface_language() {
    let sb = Sandbox::new();
    let help = |lang: &str, args: &[&str]| {
        let out = sb
            .cmd()
            .env("ADOC_LANG", lang)
            .args(args)
            .assert()
            .success();
        String::from_utf8_lossy(&out.get_output().stdout).to_string()
    };
    let en = help("en", &["--help"]);
    assert!(en.contains("Usage:") && en.contains("Commands:"), "{en}");
    assert!(en.contains("Options:") && en.contains("Print help"), "{en}");
    assert!(
        en.contains("Deterministic inventory of the repository"),
        "{en}"
    );
    assert!(!en.contains("Inventario determinista"), "{en}");

    let es = help("es", &["--help"]);
    assert!(es.contains("Uso:") && es.contains("Comandos:"), "{es}");
    assert!(
        es.contains("Opciones:") && es.contains("Muestra la ayuda"),
        "{es}"
    );
    assert!(
        es.contains("Inventario determinista del repositorio"),
        "{es}"
    );
    assert!(!es.contains("Usage:") && !es.contains("Print help"), "{es}");

    // --lang manda sobre ADOC_LANG; subcomandos anidados y `help <cmd>`.
    let out = help("es", &["--lang", "en", "task", "answer", "--help"]);
    assert!(
        out.contains("Arguments:") && out.contains("Task ID"),
        "{out}"
    );
    let out = help("es", &["help", "check"]);
    assert!(out.contains("Lista de lo hecho y lo que falta"), "{out}");
    let out = help("en", &["agent", "--help"]);
    assert!(out.contains("Change the agent"), "{out}");
}

/// Mensajes de los crates (registro, OpenSpec) en el idioma de la interfaz:
/// la validación estructural de cambios aparece en `check`.
#[test]
fn crate_messages_follow_the_interface_language() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let change = sb.root.join("openspec/changes/add-x");
    fs::create_dir_all(&change).unwrap();
    let (text, _) = run_en(&sb, &["check"], true);
    assert!(text.contains("proposal.md is missing"), "{text}");
    assert!(!text.contains("falta proposal.md"), "{text}");
}

// --- status / check / sync: salida de terminal ---

/// Ejecuta con un idioma de interfaz y devuelve (código, stdout).
fn run_lang(sb: &Sandbox, lang: &str, args: &[&str]) -> (i32, String) {
    let out = sb.cmd().env("ADOC_LANG", lang).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

/// Repositorio git con un commit inicial (para `status` y `sync`).
fn git_init(root: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    ] {
        let ok = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
}

#[test]
fn status_text_shows_sections_and_next_step() {
    let sb = Sandbox::new();
    copy_dir(&fixture_dir("medium-ts"), &sb.root);
    git_init(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);

    let (code, es) = run_lang(&sb, "es", &["status"]);
    assert_eq!(code, 0);
    for s in [
        "Proyecto",
        "Tipo",
        "Node.js",
        "TypeScript",
        "Lenguajes",
        "líneas de código",
        "Git",
        "main",
        "Estructura generada",
        "knowledge/",
        "AGENTS.md ✓",
        "Progreso",
        "Tareas pendientes",
        "T-001",
        "Agente",
        "manual",
        "Siguiente paso",
        "→ adoc inspect",
    ] {
        assert!(es.contains(s), "falta `{s}`:\n{es}");
    }
    // Sin terminal: sin códigos ANSI.
    assert!(!es.contains('\u{1b}'), "{es}");

    let (_, en) = run_lang(&sb, "en", &["status"]);
    for s in [
        "Project",
        "Generated structure",
        "Pending tasks",
        "Next step",
        "→ adoc inspect",
    ] {
        assert!(en.contains(s), "missing `{s}`:\n{en}");
    }
    // El JSON se mantiene para la extensión.
    let (_, json) = run_lang(&sb, "en", &["status", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["tasks"].as_array().unwrap().len() >= 2);
    assert_eq!(v["project"]["has_inventory"], true);
}

#[test]
fn status_text_without_scan_suggests_scan() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let (code, out) = run_lang(&sb, "es", &["status"]);
    assert_eq!(code, 0);
    assert!(out.contains("sin escanear todavía"), "{out}");
    assert!(
        out.contains("Siguiente paso") && out.contains("→ adoc scan"),
        "{out}"
    );
    assert!(!out.contains("Tareas pendientes"), "{out}");

    // Sin proyecto: cómo empezar.
    let tmp = tempfile::tempdir().unwrap();
    let out = adoc_cmd("adoc", &sb.config)
        .current_dir(tmp.path())
        .arg("status")
        .assert()
        .success();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(out.contains("→ adoc init"), "{out}");
}

#[test]
fn check_text_lists_checks_with_marks_and_hints() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    // Sin scan: inventario y overview pendientes, tareas omitidas.
    let (code, out) = run_lang(&sb, "en", &["check"]);
    assert_eq!(code, 0, "{out}");
    for s in [
        "✓ Configuration",
        "! Inventory",
        "→ adoc scan",
        "– Tasks",
        "✓ Frontmatter",
        "✓ Wikilinks",
        "– OpenSpec",
        " ok · ",
        " warnings · 0 errors",
    ] {
        assert!(out.contains(s), "missing `{s}`:\n{out}");
    }

    sb.run(&["scan"]);
    sb.run(&["plan"]);
    fs::write(
        sb.root.join("knowledge/features/f.md"),
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:1\n---\n\nVer [[nada]].\n",
    )
    .unwrap();
    let (code, out) = run_lang(&sb, "es", &["check"]);
    // Un wikilink roto es una advertencia: no cambia el código de salida.
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("✓ Inventario") && out.contains("✓ Tareas"),
        "{out}"
    );
    assert!(out.contains("! Wikilinks"), "{out}");
    assert!(out.contains("1 rotos"), "{out}");
    assert!(out.contains("knowledge/features/f.md:9 — "), "{out}");
    assert!(out.contains("→ crea la nota destino"), "{out}");
    assert!(
        out.contains("✓ Evidencias") && out.contains("1 evidencias citadas"),
        "{out}"
    );
    // Con --strict, la advertencia falla (como antes).
    let (code, out) = run_lang(&sb, "es", &["check", "--strict"]);
    assert_eq!(code, 1, "{out}");
}

#[test]
fn check_json_has_stable_checks() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    fs::write(
        sb.root.join("knowledge/features/f.md"),
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:40-50\n---\n[[nada]]\n",
    )
    .unwrap();
    let out = sb
        .cmd()
        .args(["check", "--format", "json"])
        .assert()
        .failure();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    // Claves anteriores.
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"], 1);
    assert!(v["issues"].as_array().unwrap().len() >= 2);
    assert_eq!(v["exit_code"], 1);
    let checks = v["checks"].as_array().unwrap();
    let ids: Vec<&str> = checks.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for id in [
        "config",
        "inventory",
        "tasks",
        "overview",
        "frontmatter",
        "evidence",
        "wikilinks",
        "duplicates",
        "stale",
        "agents_md",
        "site",
        "openspec",
    ] {
        assert!(ids.contains(&id), "{id}: {ids:?}");
    }
    for c in checks {
        assert!(["ok", "warn", "error", "skip"].contains(&c["status"].as_str().unwrap()));
        assert!(c["message"].is_string() && c["details"].is_array(), "{c}");
        assert!(c.get("count").is_some() && c.get("hint").is_some(), "{c}");
    }
    let by_id = |id: &str| checks.iter().find(|c| c["id"] == id).unwrap();
    assert_eq!(by_id("evidence")["status"], "error");
    assert_eq!(
        by_id("evidence")["details"][0]["path"],
        "knowledge/features/f.md"
    );
    assert_eq!(by_id("wikilinks")["status"], "warn");
    assert_eq!(by_id("wikilinks")["details"][0]["line"], 8);
    assert_eq!(by_id("tasks")["status"], "warn");
    assert_eq!(by_id("tasks")["hint"], "adoc plan");
    assert_eq!(v["summary"]["errors"], 1);
}

#[test]
fn sync_without_notes_explains_and_lists_uncovered_changes() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    git_init(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    // mtime posterior al scan (resolución de segundos en algunos FS).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(sb.root.join("src/main.py"), "def main():\n    return 2\n").unwrap();
    fs::write(sb.root.join("src/extra.py"), "X = 1\n").unwrap();

    let (code, es) = run_lang(&sb, "es", &["sync"]);
    assert_eq!(code, 0);
    assert!(es.contains("ninguna nota cita código todavía"), "{es}");
    assert!(es.contains("Cambios fuera de la documentación (2)"), "{es}");
    assert!(
        es.contains("M src/main.py") && es.contains("A src/extra.py"),
        "{es}"
    );
    assert!(es.contains("→ adoc scan && adoc plan"), "{es}");
    // Los archivos de archeology-doc no cuentan como cambios.
    assert!(
        !es.contains("AGENTS.md") && !es.contains("knowledge/"),
        "{es}"
    );

    let (_, en) = run_lang(&sb, "en", &["sync", "--dry-run"]);
    assert!(en.contains("DRY-RUN"), "{en}");
    assert!(en.contains("Changes outside the documentation (2)"), "{en}");
    assert!(en.contains("Next step"), "{en}");

    let (_, json) = run_lang(&sb, "en", &["sync", "--format", "json", "--dry-run"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["compared_against"]["mode"], "hashes");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["watched"]["files"], 0);
    assert_eq!(v["watched"]["notes"], 0);
    assert_eq!(v["changed"].as_array().unwrap().len(), 0);
    assert_eq!(v["uncovered_method"], "git");
    let uncovered = v["uncovered"].as_array().unwrap();
    assert!(uncovered
        .iter()
        .any(|u| u["path"] == "src/extra.py" && u["change"] == "added"));
    assert!(v["stale_notes"].is_array() && v["reopened_tasks"].is_array());
    assert_eq!(v["next_step"]["command"], "adoc scan && adoc plan");
}

#[test]
fn sync_marks_cited_change_and_dry_run_keeps_everything() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    git_init(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "manual"]);
    let note = sb.root.join("knowledge/modules/src.md");
    fs::write(
        &note,
        "---\nid: mod-src\ntype: module\ntitle: src\nstatus: draft\nsources:\n  - src/main.py:1-2\n---\n",
    )
    .unwrap();
    sb.run(&["task", "done", "T-002"]);
    sb.run(&["sync"]); // línea base de hashes
    fs::write(sb.root.join("src/main.py"), "def main():\n    return 1\n").unwrap();

    // dry-run: informa, pero no toca notas ni tareas.
    let before = fs::read_to_string(&note).unwrap();
    let (_, json) = run_lang(&sb, "en", &["sync", "--dry-run", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["changed"][0]["path"], "src/main.py");
    assert_eq!(v["changed"][0]["change"], "modified");
    assert_eq!(v["changed"][0]["notes"][0], "knowledge/modules/src.md");
    assert_eq!(v["stale_notes"][0], "knowledge/modules/src.md");
    assert_eq!(v["reopened_tasks"][0], "T-002");
    assert_eq!(fs::read_to_string(&note).unwrap(), before);
    assert!(!sb
        .run(&["task", "list", "--status", "pending"])
        .contains("T-002"));

    let (_, en) = run_lang(&sb, "en", &["sync"]);
    assert!(en.contains("Changes in documented sources (1)"), "{en}");
    assert!(en.contains("M src/main.py"), "{en}");
    assert!(en.contains("→ 1 notes: modules/src"), "{en}");
    assert!(
        en.contains("Tasks reopened") && en.contains("T-002"),
        "{en}"
    );
    assert!(fs::read_to_string(&note).unwrap().contains("status: stale"));
    assert!(sb
        .run(&["task", "list", "--status", "pending"])
        .contains("T-002"));
}

// --- inspect: validación y formato ---

/// Proyecto con todas las tareas hechas por el agente falso. Con `stale`,
/// además cambia la fuente de las notas y `sync` las marca desactualizadas
/// (y reabre sus tareas). `retries`: reintentos por tarea.
#[cfg(unix)]
fn inspected_fixture(stale: bool, retries: u32) -> Sandbox {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let toml = sb.root.join("archeology-doc.toml");
    let cfg = fs::read_to_string(&toml).unwrap();
    assert!(cfg.contains("retries = 1"), "{cfg}");
    fs::write(
        &toml,
        cfg.replace("retries = 1", &format!("retries = {retries}")),
    )
    .unwrap();
    add_adapter(&sb, "fake", FAKE_AGENT);
    // Agente que no toca nada (como el que solo verificó y no editó).
    add_adapter(
        &sb,
        "noop",
        "echo 'Verifiqué las notas contra el código y siguen siendo correctas. No modifiqué nada.'",
    );
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let out = sb.run(&["inspect", "--agent", "fake"]);
    assert!(!out.contains("✗"), "{out}");
    let task = fs::read_to_string(sb.work().join("tasks/T-002.md")).unwrap();
    assert!(task.contains("- `knowledge/modules/src.md`"), "{task}");
    if stale {
        // Solo la nota de T-002 cita `src/main.py`: solo ella queda desactualizada.
        fs::write(
            sb.root.join("knowledge/modules/src.md"),
            "---\nid: mod-src\ntype: module\ntitle: src\nstatus: draft\nupdated: 2026-10-01\n\
             sources:\n  - src/main.py:1\n---\n\n# src\n\nPunto de entrada `main`.\n",
        )
        .unwrap();
        sb.run(&["sync"]);
        fs::write(sb.root.join("src/main.py"), "def main():\n    return 1\n").unwrap();
        let out = sb.run(&["sync"]);
        assert!(
            out.contains("Tareas reabiertas") && out.contains("T-002"),
            "{out}"
        );
        assert!(!out.contains("T-001"), "{out}");
    }
    sb
}

/// Salidas que ya existen, válidas y vigentes: re-ejecutar la tarea sin
/// tocarlas no es un fallo (antes: "archivo no actualizado por la tarea").
#[cfg(unix)]
#[test]
fn inspect_accepts_untouched_outputs_that_are_not_stale() {
    let sb = inspected_fixture(false, 1);
    let out = sb.run(&["task", "run", "T-002", "--agent", "noop"]);
    assert!(out.contains("✓ T-002"), "{out}");
    assert!(!out.contains("✗"), "{out}");
    assert!(
        out.contains("Resumen") && out.contains("1 completada"),
        "{out}"
    );
    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-002    done"), "{list}");
    // La tarea dice qué salidas ya están documentadas.
    let task = fs::read_to_string(sb.work().join("tasks/T-002.md")).unwrap();
    assert!(task.contains("# Re-verificación"), "{task}");
    assert!(task.contains("Ya documentadas y vigentes"), "{task}");
}

/// Nota desactualizada sin re-verificar: falla con un mensaje claro, el
/// reintento recibe la retroalimentación y, si el fallo se repite igual,
/// no se gastan más intentos.
#[cfg(unix)]
#[test]
fn inspect_fails_stale_untouched_output_with_feedback_and_stops_retrying() {
    let sb = inspected_fixture(true, 3);
    let task_path = sb.work().join("tasks/T-002.md");
    // `sync` ya escribió en la tarea qué nota re-verificar y por qué.
    let task = fs::read_to_string(&task_path).unwrap();
    assert!(task.contains("# Re-verificación"), "{task}");
    assert!(
        task.contains("`knowledge/modules/src.md` — cambió: `src/main.py`"),
        "{task}"
    );
    assert!(
        task.contains("cambia `status: stale` por `status: draft`"),
        "{task}"
    );

    // Agente que guarda cada prompt recibido y no toca nada.
    let capture = sb.root.parent().unwrap().join("prompts.txt");
    let script = format!(
        "cat \"$1\" >> '{}'; echo '=====' >> '{}'; echo 'Las notas siguen vigentes. No cambié nada.'",
        capture.display(),
        capture.display()
    );
    add_adapter(&sb, "eco", &script);
    let out = sb.run(&["inspect", "--agent", "eco", "--task", "T-002"]);

    // Una línea por tarea con el problema, el mensaje del agente y cómo seguir.
    assert!(out.contains("✗ T-002"), "{out}");
    assert!(
        out.contains("el agente no re-verificó 1 nota desactualizada: src.md"),
        "{out}"
    );
    assert!(out.contains("“Las notas siguen vigentes.”"), "{out}");
    assert!(out.contains("→ re-verifica las notas"), "{out}");
    assert!(out.contains("no se reintentó más"), "{out}");
    // El texto largo no se repite en el resumen.
    assert_eq!(out.matches("re-verificó").count(), 1, "{out}");
    assert!(
        out.contains("Resumen") && out.contains("1 fallida"),
        "{out}"
    );
    assert!(out.contains("Siguiente"), "{out}");
    assert!(out.contains("→ adoc task show T-002"), "{out}");

    // Dos intentos (no cuatro): el segundo falló igual que el primero.
    let prompts = fs::read_to_string(&capture).unwrap();
    let attempts: Vec<&str> = prompts
        .split("=====")
        .filter(|p| !p.trim().is_empty())
        .collect();
    assert_eq!(attempts.len(), 2, "{prompts}");
    assert!(!attempts[0].contains("# Validación del intento anterior"));
    assert!(
        attempts[1].contains("# Validación del intento anterior"),
        "{}",
        attempts[1]
    );
    assert!(
        attempts[1].contains("`knowledge/modules/src.md`: desactualizada y sin re-verificar"),
        "{}",
        attempts[1]
    );
    let st: serde_json::Value =
        serde_json::from_str(&sb.run(&["status", "--format", "json"])).unwrap();
    let t2 = st["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "T-002")
        .unwrap();
    assert_eq!(t2["status"], "failed", "{t2}");
    assert_eq!(t2["last_run"]["attempts"], 2, "{t2}");

    // run.log y el log de la tarea usan los mensajes claros.
    let run = fs::read_dir(sb.work().join("runs"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .max()
        .unwrap();
    let log = fs::read_to_string(run.join("run.log")).unwrap();
    assert!(
        log.contains("el agente no re-verificó 1 nota desactualizada"),
        "{log}"
    );
    let tlog = fs::read_to_string(run.join("T-002.log")).unwrap();
    assert!(
        tlog.contains("nota desactualizada sin re-verificar"),
        "{tlog}"
    );

    // `task show` muestra el último fallo con el mismo formato.
    let show = sb.run(&["task", "show", "T-002"]);
    assert!(show.contains("Último fallo"), "{show}");
    assert!(
        show.contains("el agente no re-verificó 1 nota desactualizada"),
        "{show}"
    );
    assert!(show.contains("T-002.log"), "{show}");

    // En inglés, el mismo formato.
    let (en, _) = run_en(
        &sb,
        &["inspect", "--agent", "noop", "--resume", "--task", "T-002"],
        true,
    );
    assert!(en.contains("✗ T-002"), "{en}");
    assert!(
        en.contains("the agent didn't re-verify 1 stale note: src.md"),
        "{en}"
    );
    assert!(en.contains("Summary") && en.contains("1 failed"), "{en}");
    assert!(
        en.contains("Next step") && en.contains("adoc task show T-002"),
        "{en}"
    );
    assert_eq!(en.matches("re-verify 1 stale note").count(), 1, "{en}");
}

/// Re-verificar sin reescribir: el agente actualiza `updated` y quita
/// `status: stale`. La tarea termina y la nota deja de figurar como
/// desactualizada en el estado.
#[cfg(unix)]
#[test]
fn inspect_accepts_reverified_stale_output_and_clears_state() {
    let sb = inspected_fixture(true, 1);
    let state = || -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(sb.work().join("state.json")).unwrap()).unwrap()
    };
    let note = "knowledge/modules/src.md";
    assert!(state()["stale_notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n == note));
    assert_eq!(state()["stale_reasons"][note][0], "src/main.py");

    // Conserva el contenido: solo `updated` y `status: stale` → `draft`.
    let reverify = "f=knowledge/modules/src.md; awk '{ if ($0 == \"status: stale\") print \"status: draft\"; else if ($0 ~ /^updated: /) print \"updated: 2026-10-14\"; else print }' $f > $f.tmp && mv $f.tmp $f; echo 'Re-verificada.'";
    add_adapter(&sb, "verifica", reverify);
    let out = sb.run(&["inspect", "--agent", "verifica", "--task", "T-002"]);
    assert!(out.contains("✓ T-002"), "{out}");
    assert!(out.contains("1 completada"), "{out}");
    assert!(!out.contains("✗"), "{out}");

    let text = fs::read_to_string(sb.root.join(note)).unwrap();
    assert!(text.contains("status: draft"), "{text}");
    let st = state();
    assert!(
        !st["stale_notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == note),
        "{st}"
    );
    assert!(st["stale_reasons"].get(note).is_none(), "{st}");
    // La tarea ya no pide re-verificar esa nota.
    let task = fs::read_to_string(sb.work().join("tasks/T-002.md")).unwrap();
    assert!(
        !task.contains("`knowledge/modules/src.md` — cambió"),
        "{task}"
    );
    assert!(
        !task.contains("# Validación del intento anterior"),
        "{task}"
    );
}

/// Formato de una corrida correcta en inglés: encabezado, una línea por
/// tarea, resumen y siguiente paso.
#[cfg(unix)]
#[test]
fn inspect_output_format_success_in_english() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    add_adapter(&sb, "fake", FAKE_AGENT);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let (out, _) = run_en(&sb, &["inspect", "--agent", "fake"], true);
    let first = out.lines().next().unwrap_or_default();
    assert!(
        first.contains(" · fake · ") && first.contains("tasks to run"),
        "{out}"
    );
    assert!(out.contains("✓ T-001  overview"), "{out}");
    assert!(
        out.contains("Summary") && out.contains("completed"),
        "{out}"
    );
    assert!(!out.contains("failed"), "{out}");
    assert!(out.contains("Logs") && out.contains("runs/"), "{out}");
    assert_eq!(out.matches("Next step").count(), 1, "{out}");
}

// --- evidencias fuera de rango ---

/// `check` sugiere `--fix` para los rangos que solo se pasan del final del
/// archivo; `check --fix` los ajusta, lo informa y deja solo lo irreparable.
#[test]
fn check_fix_clamps_evidence_past_end_of_file() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    // src/main.py tiene 2 líneas.
    let note = sb.root.join("knowledge/features/f.md");
    fs::write(
        &note,
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:1-10\n  - README.md:1\n---\n# F\n",
    )
    .unwrap();
    let out = sb.cmd().arg("check").assert().failure();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(out.contains("✗ Evidencias"), "{out}");
    assert!(out.contains("1 se puede ajustar automáticamente"), "{out}");
    assert!(out.contains("→ adoc check --fix"), "{out}");
    // Rutas relativas a la raíz del proyecto, nunca absolutas.
    assert!(
        out.contains("knowledge/features/f.md — evidencia fuera de rango: src/main.py:1-10"),
        "{out}"
    );
    assert!(!out.contains(&sb.root.display().to_string()), "{out}");

    let out = sb.run(&["check", "--fix"]);
    assert!(out.contains("✓ Evidencias"), "{out}");
    assert!(
        out.contains("1 evidencia ajustada al final del archivo"),
        "{out}"
    );
    assert!(
        out.contains(
            "knowledge/features/f.md — ajustado: src/main.py:1-10 → 1-2 (el archivo tiene 2 líneas)"
        ),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(&note).unwrap(),
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:1-2\n  - README.md:1\n---\n# F\n"
    );
    // Ya no queda nada que ajustar.
    let out = sb.run(&["check"]);
    assert!(!out.contains("--fix"), "{out}");

    // Inicio más allá del final: --fix no lo toca y sigue fallando.
    fs::write(
        &note,
        "---\nid: feat-f\ntype: feature\ntitle: f\nsources:\n  - src/main.py:5-10\n---\n",
    )
    .unwrap();
    let out = sb.cmd().args(["check", "--fix"]).assert().failure();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        out.contains("evidencia fuera de rango: src/main.py:5-10"),
        "{out}"
    );
    assert!(!out.contains("ajustado"), "{out}");
}

/// Una tarea cuyas notas citan rangos redondeados hacia arriba (`1-50` en un
/// archivo de 1 línea) termina bien: el rango se ajusta y queda como aviso
/// en la salida de `inspect`, el log de la tarea y run.log.
#[cfg(unix)]
#[test]
fn inspect_adjusts_clampable_evidence_instead_of_failing() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    add_adapter(
        &sb,
        "redondea",
        &FAKE_AGENT.replace("README.md:1", "README.md:1-50"),
    );
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    let out = sb.run(&["inspect", "--agent", "redondea"]);
    assert!(!out.contains("✗"), "{out}");
    assert!(
        out.contains("! ajustado: README.md:1-50 → 1 (el archivo tiene 1 líneas)"),
        "{out}"
    );
    assert!(!sb.run(&["task", "list"]).contains("failed"));
    let text = fs::read_to_string(sb.root.join("knowledge/modules/src.md")).unwrap();
    assert!(text.contains("  - README.md:1\n"), "{text}");
    let run = fs::read_dir(sb.work().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let log = fs::read_to_string(run.join("run.log")).unwrap();
    assert!(log.contains("ajustado: README.md:1-50 → 1"), "{log}");
    let task_log = fs::read_to_string(run.join("T-001.log")).unwrap();
    assert!(task_log.contains("ajustado: README.md:1-50"), "{task_log}");
    sb.run(&["check"]);
}

/// `task done` también ajusta y muestra rutas relativas.
#[test]
fn task_done_adjusts_clampable_evidence() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes"]);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    sb.run(&["inspect", "--agent", "manual"]);
    fs::write(
        sb.root.join("knowledge/modules/src.md"),
        "---\nid: mod-src\ntype: module\ntitle: src\nsources:\n  - src/main.py:2-9\n---\n",
    )
    .unwrap();
    let out = sb.cmd().args(["task", "done", "T-002"]).assert().success();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(
        err.contains(
            "knowledge/modules/src.md: ajustado: src/main.py:2-9 → 2 (el archivo tiene 2 líneas)"
        ),
        "{err}"
    );
    assert!(!err.contains(&sb.root.display().to_string()), "{err}");
}

// --- graph/init/projects delete ---

/// Proyectos del registro aislado del sandbox (`projects list --format json`).
fn registry_rows(sb: &Sandbox) -> Vec<serde_json::Value> {
    serde_json::from_str(&sb.run(&["projects", "list", "--format", "json"])).unwrap()
}

/// `adoc graph` sin flags genera el HTML en el espacio de trabajo e intenta
/// abrirlo (aquí desactivado con `ADOC_NO_OPEN`); `--json` y el antiguo
/// `--format json` imprimen el JSON.
#[test]
fn graph_defaults_to_html_and_json_flags_print_json() {
    let sb = Sandbox::new();
    graph_fixture(&sb);
    let out = sb
        .cmd()
        .env("ADOC_NO_OPEN", "1")
        .arg("graph")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let html = sb.work().join("graph.html");
    assert!(html.is_file(), "{stdout}");
    assert!(stdout.contains("graph.html"), "{stdout}");
    fs::remove_file(&html).unwrap();
    sb.run(&["graph", "--no-open"]);
    assert!(html.is_file());

    let g: serde_json::Value = serde_json::from_str(&sb.run(&["graph", "--json"])).unwrap();
    assert!(g["nodes"].as_array().is_some_and(|n| !n.is_empty()));
    let old: serde_json::Value =
        serde_json::from_str(&sb.run(&["graph", "--format", "json"])).unwrap();
    assert_eq!(old["nodes"], g["nodes"]);

    let file = sb.root.parent().unwrap().join("g.json");
    sb.run(&["graph", "--json", "--out", file.to_str().unwrap()]);
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(written["nodes"], g["nodes"]);
    // `--json` con un flag de HTML es un error de uso.
    sb.cmd()
        .args(["graph", "--json", "--no-open"])
        .assert()
        .failure();
}

#[test]
fn init_twice_says_already_initialized() {
    let sb = Sandbox::new();
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let cfg = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    let out = sb
        .cmd()
        .args(["init", "--yes", "--agent", "manual"])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("ya está inicializado"), "{err}");
    assert!(
        err.contains("adoc status") && err.contains("--force"),
        "{err}"
    );
    // Nada se sobrescribió ni se duplicó en el registro.
    assert_eq!(
        fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap(),
        cfg
    );
    assert_eq!(registry_rows(&sb).len(), 1);
    // `--force` reinicializa conservando el ID.
    let id = registry_rows(&sb)[0]["id"].clone();
    sb.run(&["init", "--yes", "--agent", "manual", "--force"]);
    let rows = registry_rows(&sb);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id);
}

#[test]
fn init_rejects_same_name_at_other_path_and_name_flag_fixes_it() {
    let sb = Sandbox::new();
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let other = sb.root.parent().unwrap().join("b").join("proj");
    fs::create_dir_all(&other).unwrap();
    let out = sb
        .cmd()
        .args(["init", "--yes", "--agent", "manual", "--dir"])
        .arg(&other)
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("'proj'") && err.contains("--name"), "{err}");
    assert!(!other.join("archeology-doc.toml").exists());

    sb.cmd()
        .args([
            "init", "--yes", "--agent", "manual", "--name", "proj2", "--dir",
        ])
        .arg(&other)
        .assert()
        .success();
    let names: Vec<String> = registry_rows(&sb)
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"proj".into()) && names.contains(&"proj2".into()));

    // Si la carpeta del proyecto registrado ya no existe, se sugiere borrarlo.
    fs::remove_dir_all(&other).unwrap();
    let third = sb.root.parent().unwrap().join("c").join("proj2");
    fs::create_dir_all(&third).unwrap();
    let out = sb
        .cmd()
        .args(["init", "--yes", "--agent", "manual", "--dir"])
        .arg(&third)
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("adoc projects delete"), "{err}");
}

/// Carpeta copiada (mismo ID): aviso y `--new-id` le da su propio ID.
#[test]
fn init_on_copied_folder_suggests_new_id() {
    let sb = Sandbox::new();
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let copy = sb.root.parent().unwrap().join("copia");
    fs::create_dir_all(&copy).unwrap();
    fs::copy(
        sb.root.join("archeology-doc.toml"),
        copy.join("archeology-doc.toml"),
    )
    .unwrap();
    let out = sb
        .cmd()
        .args(["init", "--yes", "--dir"])
        .arg(&copy)
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("--new-id"), "{err}");
    // El original sigue registrado en su ruta.
    let rows = registry_rows(&sb);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0]["path"].as_str().unwrap().contains("copia"));

    sb.cmd()
        .args(["init", "--new-id", "--dir"])
        .arg(&copy)
        .assert()
        .success();
    let rows = registry_rows(&sb);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0]["id"], rows[1]["id"]);
    let cfg = fs::read_to_string(copy.join("archeology-doc.toml")).unwrap();
    assert!(
        cfg.contains(rows[1]["id"].as_str().unwrap())
            || cfg.contains(rows[0]["id"].as_str().unwrap())
    );
}

#[test]
fn projects_delete_yes_removes_registry_and_workspace_but_keeps_files() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.run(&["scan"]);
    let work = sb.work();
    let out = sb.run(&["projects", "delete", "proj", "--yes"]);
    assert!(
        out.contains("Se eliminará") && out.contains("espacio de trabajo"),
        "{out}"
    );
    assert!(registry_rows(&sb).is_empty());
    assert!(!work.exists());
    assert!(sb.root.join("archeology-doc.toml").is_file());
    assert!(sb.root.join("knowledge").is_dir());
    assert!(sb.root.join("AGENTS.md").is_file());
}

#[test]
fn projects_delete_files_removes_generated_but_keeps_user_content() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual", "--adapters", "claude"]);
    sb.run(&["scan"]);
    assert!(sb.root.join("CLAUDE.md").is_file() && sb.root.join(".agents").is_dir());
    // Texto propio en AGENTS.md y una página propia en docs/.
    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    fs::write(
        sb.root.join("AGENTS.md"),
        format!("{agents}\n## Mis reglas\n\nNo tocar.\n"),
    )
    .unwrap();
    fs::write(sb.root.join("docs/guides/mia.md"), "# Mía\n").unwrap();

    let out = sb.run(&["projects", "delete", "proj", "-y", "-d"]);
    assert!(out.contains("knowledge/"), "{out}");
    assert!(registry_rows(&sb).is_empty());
    for gone in [
        "archeology-doc.toml",
        "knowledge",
        "llms.txt",
        "CLAUDE.md",
        ".agents",
    ] {
        assert!(!sb.root.join(gone).exists(), "{gone} sigue: {out}");
    }
    let agents = fs::read_to_string(sb.root.join("AGENTS.md")).unwrap();
    assert!(agents.contains("No tocar."), "{agents}");
    assert!(!agents.contains("archeology-doc:begin"), "{agents}");
    assert!(sb.root.join("docs/guides/mia.md").is_file());
    assert!(sb.root.join("src/main.py").is_file());
}

/// Sin páginas propias, el `docs/` que creó `init` se borra completo.
#[test]
fn projects_delete_files_removes_docs_created_by_init() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    assert!(sb.root.join("docs").is_dir());
    sb.run(&["projects", "delete", "proj", "--yes", "--delete-files"]);
    assert!(!sb.root.join("docs").exists());
    assert!(!sb.root.join("AGENTS.md").exists());
    assert!(sb.root.join("src/main.py").is_file());
}

#[test]
fn projects_delete_without_yes_and_no_tty_fails() {
    let sb = Sandbox::new();
    sb.run(&["init", "--yes", "--agent", "manual"]);
    sb.cmd()
        .args(["projects", "delete", "proj"])
        .assert()
        .failure();
    assert_eq!(registry_rows(&sb).len(), 1);
    assert!(sb.work().is_dir());
}

// --- límites de agentes y modelos ---

/// Proyecto planificado con un agente `nombre` que, en la tarea `task`,
/// imprime `message` y sale con `code` (en las demás hace el trabajo como el
/// agente falso). Cada invocación deja una línea en `calls.txt`.
#[cfg(unix)]
fn limited_agent_fixture(
    name: &str,
    task: &str,
    message: &str,
    code: i32,
) -> (Sandbox, std::path::PathBuf) {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let calls = sb.root.parent().unwrap().join("calls.txt");
    let script = format!(
        "echo \"$1\" >> '{calls}'\ncase \"$1\" in *{task}.md) printf '%s\\n' 'trabajando...' '{message}' >&2; exit {code};; esac\n{FAKE_AGENT}",
        calls = calls.display()
    );
    add_adapter(&sb, name, &script);
    sb.run(&["scan"]);
    sb.run(&["plan"]);
    (sb, calls)
}

#[cfg(unix)]
fn calls(path: &Path) -> usize {
    fs::read_to_string(path).unwrap_or_default().lines().count()
}

/// Límite de uso (Claude Code): sin reintentos, la tarea vuelve a `pending`,
/// la corrida se detiene y sale con 75 explicando el reinicio y qué hacer.
#[cfg(unix)]
#[test]
fn inspect_stops_run_on_agent_usage_limit() {
    let (sb, calls_file) = limited_agent_fixture(
        "tope",
        "T-002",
        "Claude AI usage limit reached|1760466000",
        1,
    );
    let out = sb
        .cmd()
        .args(["inspect", "--agent", "tope"])
        .assert()
        .code(75);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("✓ T-001"), "{stdout}");
    assert!(stdout.contains("detenida: límite de uso"), "{stdout}");
    assert!(
        stdout.contains("El agente alcanzó un límite · tope (límite de uso)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("“Claude AI usage limit reached|1760466000”"),
        "{stdout}"
    );
    assert!(stdout.contains("hora local"), "{stdout}");
    assert!(stdout.contains("código de salida 75"), "{stdout}");
    assert!(stdout.contains("adoc inspect --resume"), "{stdout}");
    assert!(stdout.contains("adoc agent use"), "{stdout}");
    assert!(stdout.contains("--agent"), "{stdout}");
    assert!(!stdout.contains("✗"), "{stdout}");
    // Una llamada para T-001 y una sola para T-002: ni reintentos ni más tareas.
    assert_eq!(calls(&calls_file), 2);
    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-002    pending"), "{list}");
    assert!(!list.contains("failed"), "{list}");
    // Clasificación en run.log y en el log de la tarea.
    let run = fs::read_dir(sb.work().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let log = fs::read_to_string(run.join("run.log")).unwrap();
    assert!(log.contains("corrida detenida (usage_limit)"), "{log}");
    let task_log = fs::read_to_string(run.join("T-002.log")).unwrap();
    assert!(task_log.contains("[límite] usage_limit"), "{task_log}");
    assert!(task_log.contains("2025-10-14T18:20:00+00:00"), "{task_log}");
}

/// Sin sesión: se detiene con 77 y dice cómo iniciar sesión.
#[cfg(unix)]
#[test]
fn inspect_stops_run_when_agent_is_not_logged_in() {
    let (sb, calls_file) = limited_agent_fixture(
        "sinsesion",
        "T-001",
        "Invalid API key · Please run /login",
        1,
    );
    let out = sb
        .cmd()
        .args(["inspect", "--agent", "sinsesion"])
        .assert()
        .code(77);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("El agente no tiene sesión iniciada · sinsesion"),
        "{stdout}"
    );
    assert!(stdout.contains("Inicia sesión:"), "{stdout}");
    assert!(stdout.contains("Luego: adoc inspect --resume"), "{stdout}");
    assert_eq!(calls(&calls_file), 1);
    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-001    pending"), "{list}");
}

/// Contexto excedido: la tarea falla (sin reintentar) con la sugerencia de
/// dividirla y la corrida sigue.
#[cfg(unix)]
#[test]
fn inspect_context_limit_fails_task_with_split_hint_and_continues() {
    let (sb, calls_file) = limited_agent_fixture(
        "grande",
        "T-002",
        "API Error: 400 prompt is too long: 215000 tokens > 200000 maximum",
        1,
    );
    let out = sb.run(&["inspect", "--agent", "grande"]);
    assert!(out.contains("✗ T-002"), "{out}");
    assert!(out.contains("límite de contexto del agente"), "{out}");
    assert!(out.contains("max_files_per_task (ahora 40)"), "{out}");
    assert!(out.contains("adoc plan --force"), "{out}");
    // Sin bloque de límite: la corrida no se detuvo.
    assert!(!out.contains("alcanzó un límite"), "{out}");
    let list = sb.run(&["task", "list"]);
    assert!(list.contains("T-002    failed"), "{list}");
    // T-002 se intentó una sola vez (sin reintento) y la corrida siguió.
    let all = fs::read_to_string(&calls_file).unwrap();
    assert_eq!(all.matches("T-002.md").count(), 1, "{all}");
}

/// opencode: el modelo va como proveedor/modelo antes del mensaje, y
/// `agent use` muestra el comando efectivo.
#[test]
fn agent_use_opencode_model_needs_provider_and_shows_command() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    // `opencode` falso delante en el PATH: el test no depende del opencode
    // instalado en la máquina (ni de qué modelos tenga configurados).
    let fake_bin = sb.root.parent().unwrap().join("fakebin");
    fs::create_dir_all(&fake_bin).unwrap();
    let path_env = if cfg!(unix) {
        let script = fake_bin.join("opencode");
        fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = models ]; then\n  echo anthropic/claude-sonnet-4-5\n  echo opencode/muse-spark-1.3-contributor-free\nfi\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!(
            "{}:{}",
            fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    } else {
        // Sin opencode en el PATH la existencia del modelo no se verifica.
        fake_bin.display().to_string()
    };
    let cmd = || {
        let mut c = sb.cmd();
        c.env("PATH", &path_env);
        c
    };
    let stdout = |args: &[&str]| {
        let out = cmd().args(args).assert().success();
        String::from_utf8_lossy(&out.get_output().stdout).to_string()
    };

    let err = cmd()
        .args(["agent", "use", "opencode", "--model", "claude-sonnet-4-5"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(stderr.contains("proveedor/modelo"), "{stderr}");
    assert!(stderr.contains("anthropic/claude-sonnet-4-5"), "{stderr}");
    assert!(stderr.contains("opencode models"), "{stderr}");
    let toml = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
    assert!(!toml.contains("claude-sonnet-4-5"), "{toml}");

    let err = cmd()
        .args(["agent", "use", "opencode", "--model", "Claude Sonnet 4.5"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(stderr.contains("no llevan espacios"), "{stderr}");

    // Formato correcto pero el modelo no existe: error con sugerencias.
    if cfg!(unix) {
        let err = cmd()
            .args([
                "agent",
                "use",
                "opencode",
                "--model",
                "opencode/muse-spark-1.3",
            ])
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&err.get_output().stderr).to_string();
        assert!(stderr.contains("no existe"), "{stderr}");
        assert!(
            stderr.contains("opencode/muse-spark-1.3-contributor-free"),
            "{stderr}"
        );
        let toml = fs::read_to_string(sb.root.join("archeology-doc.toml")).unwrap();
        assert!(!toml.contains("muse-spark"), "{toml}");
    }

    let out = stdout(&[
        "agent",
        "use",
        "opencode",
        "--model",
        "anthropic/claude-sonnet-4-5",
    ]);
    assert!(
        out.contains("Comando: opencode run --model anthropic/claude-sonnet-4-5 <prompt>"),
        "{out}"
    );
    let out = stdout(&["agent", "test", "--agent", "opencode"]);
    assert!(
        out.contains("opencode run --model anthropic/claude-sonnet-4-5 <prompt>"),
        "{out}"
    );
}

/// Un CLI que no permite elegir el modelo da un error explícito; `agent
/// models` explica cómo se pasa el modelo cuando no hay comando para listarlos.
#[test]
fn unsupported_model_agent_errors_and_models_explains() {
    let sb = Sandbox::new();
    fixture(&sb.root);
    sb.run(&["init", "--yes", "--agent", "manual"]);
    let err = sb
        .cmd()
        .args(["agent", "use", "amp", "--model", "gpt-5"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(
        stderr.contains("amp no permite elegir el modelo"),
        "{stderr}"
    );
    let out = sb.run(&["agent", "models", "claude"]);
    assert!(out.contains("`--model <modelo>` antes del prompt"), "{out}");
    assert!(out.contains("no tiene un comando para listar"), "{out}");
    let out = sb.run(&["agent", "models", "amp"]);
    assert!(out.contains("no permite elegir el modelo"), "{out}");
}

// --- context ---

/// Contrato JSON de `adoc context` (raíz del repositorio).
const CONTEXT_SCHEMA: &str = include_str!("../../../schemas/context.v1.schema.json");

/// Validador mínimo de JSON Schema (subconjunto que usa el contrato:
/// `type`, `required`, `properties`, `items`, `enum`, `minimum`, `maximum`
/// y `$ref` a `#/$defs/...`). Devuelve los errores con su ruta.
fn schema_errors(
    schema: &serde_json::Value,
    root: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    use serde_json::Value;
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.trim_start_matches("#/$defs/");
        return schema_errors(&root["$defs"][name], root, value, path, errors);
    }
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let ok = types.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !ok {
            errors.push(format!("{path}: tipo {types:?}, valor {value}"));
            return;
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(value) {
            errors.push(format!("{path}: {value} no está en {e:?}"));
        }
    }
    if let (Some(min), Some(v)) = (
        schema.get("minimum").and_then(Value::as_f64),
        value.as_f64(),
    ) {
        if v < min {
            errors.push(format!("{path}: {v} < {min}"));
        }
    }
    if let (Some(max), Some(v)) = (
        schema.get("maximum").and_then(Value::as_f64),
        value.as_f64(),
    ) {
        if v > max {
            errors.push(format!("{path}: {v} > {max}"));
        }
    }
    if let Some(obj) = value.as_object() {
        for req in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !obj.contains_key(req) {
                errors.push(format!("{path}: falta `{req}`"));
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, s) in props {
                if let Some(v) = obj.get(k) {
                    schema_errors(s, root, v, &format!("{path}.{k}"), errors);
                }
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), value.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            schema_errors(items, root, v, &format!("{path}[{i}]"), errors);
        }
    }
}

/// Falla si `value` no cumple el contrato `context.v1`.
fn assert_context_schema(value: &serde_json::Value) {
    let schema: serde_json::Value = serde_json::from_str(CONTEXT_SCHEMA).unwrap();
    let mut errors = Vec::new();
    schema_errors(&schema, &schema, value, "$", &mut errors);
    assert!(errors.is_empty(), "{errors:#?}\n{value:#}");
}

/// `adoc context` con argumentos: (código, stdout, stderr).
fn context_run(sb: &Sandbox, lang: &str, args: &[&str]) -> (i32, String, String) {
    let out = sb
        .cmd()
        .env("ADOC_LANG", lang)
        .arg("context")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// `adoc context ... --format json`, validado contra el schema.
fn context_json(sb: &Sandbox, args: &[&str]) -> (i32, serde_json::Value) {
    let mut all = args.to_vec();
    all.extend(["--format", "json"]);
    let (code, out, err) = context_run(sb, "es", &all);
    let v: serde_json::Value =
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}\n{err}"));
    assert_context_schema(&v);
    (code, v)
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x[key].as_str().unwrap_or("").to_string())
        .collect()
}

/// Proyecto con módulos, funcionalidad, hallazgos, reporte de tests,
/// decisión, glosario, duda abierta y un archivo secreto citado.
fn context_fixture(sb: &Sandbox) {
    let root = &sb.root;
    let w = |rel: &str, text: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    w("src/billing/invoice.py", &"x = 1\n".repeat(10));
    w("src/billing/tax.py", "IVA = 0.13\n");
    w("src/billing/credentials.json", "{\"token\": \"s3cr3t\"}\n");
    w(
        "src/billing/test_invoice.py",
        "import pytest\n@pytest.mark.skip\ndef test_x(): pass\n",
    );
    w("src/auth/login.py", "def login(): pass\n");
    sb.run(&["init", "--yes", "--agent", "manual"]);
    w(
        "knowledge/modules/billing.md",
        "---\nid: mod-billing\ntype: module\ntitle: Facturación\nstatus: draft\nsources: [src/billing/invoice.py:1-5]\n---\n\
         # Facturación\n\n<!-- archeology-doc:begin summary -->\nCobros, facturas e impuestos.\n<!-- archeology-doc:end summary -->\n",
    );
    w(
        "knowledge/modules/auth.md",
        "---\nid: mod-auth\ntype: module\ntitle: Autenticación\nstatus: draft\nsources: [src/auth/login.py:1]\n---\n# Autenticación\n\nInicio de sesión.\n",
    );
    w(
        "knowledge/features/checkout.md",
        "---\nid: feat-checkout\ntype: feature\ntitle: Checkout\nstatus: draft\nmodules: [billing]\n\
         sources: [src/billing/invoice.py:2-3, src/billing/credentials.json]\n---\n# Checkout\n\n\
         <!-- archeology-doc:begin summary -->\nCobra el carrito y emite la factura.\n<!-- archeology-doc:end summary -->\n\n\
         Requiere sesión: ver [[auth]].\n",
    );
    w(
        "knowledge/findings/rounding.md",
        "---\nid: find-rounding\ntype: finding\ntitle: Redondeo distinto\nkind: contradiction\nseverity: high\nstate: open\n\
         related: [feat-checkout]\nsources: [src/billing/invoice.py:4-99]\n---\n# Redondeo\n\n## Descripción\n\nEl total redondea distinto que el impuesto.\n",
    );
    w(
        "knowledge/findings/old.md",
        "---\nid: find-old\ntype: finding\ntitle: Código muerto viejo\nkind: dead-code\nseverity: low\nstate: resolved\nsources: [src/billing/tax.py:1]\n---\n# Viejo\n",
    );
    w(
        "knowledge/tests/checkout.md",
        "---\nid: test-checkout\ntype: test-report\ntitle: \"Tests: Checkout\"\nfeature: feat-checkout\ncovered_scenarios: 1\nmissing_scenarios: 1\n\
         test_files: [src/billing/test_invoice.py]\nsources: [src/billing/test_invoice.py:1]\n---\n# Cobertura\n\n## Escenarios cubiertos\n\n\
         | Escenario | Tests |\n|-----------|-------|\n| Cobro exitoso | src/billing/test_invoice.py:3 |\n\n## Escenarios faltantes\n\n\
         | Escenario | Tipo | Prioridad sugerida |\n|---|---|---|\n| Tarjeta rechazada | error | alta |\n",
    );
    w(
        "knowledge/decisions/money.md",
        "---\nid: dec-money\ntype: decision\ntitle: Montos en centavos\nsources: [src/billing/tax.py:1]\n---\n# Montos\n\n> Inferida.\n\n## Contexto\n\nErrores de redondeo.\n\n## Decisión\n\nGuardar montos en centavos.\n",
    );
    w(
        "knowledge/glossary/iva.md",
        "---\nid: glossary-iva\ntype: glossary\ntitle: IVA\nsources: [src/billing/tax.py:1]\n---\n# IVA\n\nImpuesto al valor agregado.\n",
    );
    w(
        "knowledge/memory/open-questions.md",
        "---\nid: memory-open-questions\ntype: memory\ntitle: Dudas\n---\n# Dudas\n\n- ¿El módulo billing redondea por línea o por total?\n- ¿Quién mantiene el login?\n",
    );
    sb.run(&["scan"]);
}

#[test]
fn context_resolves_path_module_feature_and_bare_targets() {
    let sb = Sandbox::new();
    context_fixture(&sb);

    // Ruta (carpeta): módulos que la contienen y notas con evidencia en ella.
    let (code, v) = context_json(&sb, &["src/billing/"]);
    assert_eq!(code, 0);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["resolved_targets"][0]["kind"], "path");
    let matches = v["resolved_targets"][0]["matches"].as_array().unwrap();
    assert!(matches.iter().any(|m| m == "module:billing"), "{v:#}");
    let notes = str_list(&v["notes"], "path");
    assert!(notes.contains(&"knowledge/features/checkout.md".to_string()));
    assert!(notes.contains(&"knowledge/glossary/iva.md".to_string()));
    // Hallazgos: solo abiertos; evidencia fuera de rango → valid:false.
    let findings = v["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{v:#}");
    assert_eq!(findings[0]["id"], "find-rounding");
    assert_eq!(findings[0]["category"], "contradiction");
    assert_eq!(findings[0]["evidence"][0]["valid"], false);
    // Tests: tabla de cubiertos/faltantes y test desactivado del inventario.
    assert_eq!(v["tests"]["covered"][0]["scenario"], "Cobro exitoso");
    assert_eq!(
        v["tests"]["covered"][0]["tests"][0],
        "src/billing/test_invoice.py:3"
    );
    assert_eq!(v["tests"]["missing"][0]["scenario"], "Tarjeta rechazada");
    assert_eq!(v["tests"]["missing"][0]["feature"], "feature:checkout");
    assert_eq!(
        v["tests"]["disabled_or_empty"][0]["file"],
        "src/billing/test_invoice.py"
    );
    assert!(v["tests"]["disabled_or_empty"][0]["line"].is_null());
    assert_eq!(v["decisions"][0]["summary"], "Guardar montos en centavos.");
    assert_eq!(v["glossary"][0]["term"], "IVA");
    assert_eq!(v["open_questions"][0]["id"], "memory#1");

    // Archivo con separadores de Windows: normalizado a `/`.
    let (code, v) = context_json(&sb, &["src\\billing\\invoice.py"]);
    assert_eq!(code, 0);
    assert_eq!(v["resolved_targets"][0]["kind"], "path");
    assert!(str_list(&v["notes"], "path").contains(&"knowledge/modules/billing.md".to_string()));
    assert_eq!(
        v["resolved_targets"][0]["input"],
        "src\\billing\\invoice.py"
    );
    let mut rest = v.clone();
    rest["resolved_targets"] = serde_json::Value::Null;
    rest["project"] = serde_json::Value::Null;
    // Una `\` literal se serializa como `\\` en JSON.
    assert!(!rest.to_string().contains("\\\\"), "{rest}");

    // Módulo y funcionalidad con prefijo; sin prefijo por título normalizado.
    let (_, v) = context_json(&sb, &["module:billing"]);
    assert_eq!(v["modules"][0]["id"], "module:billing");
    assert_eq!(v["modules"][0]["summary"], "Cobros, facturas e impuestos.");
    assert_eq!(v["notes"][0]["path"], "knowledge/modules/billing.md");
    assert_eq!(v["notes"][0]["relevance"], 1.0);
    let (_, v) = context_json(&sb, &["feature:checkout"]);
    assert_eq!(v["features"][0]["id"], "feature:checkout");
    assert_eq!(v["features"][0]["modules"][0], "module:billing");
    let (_, v) = context_json(&sb, &["facturacion"]);
    assert_eq!(v["resolved_targets"][0]["kind"], "module");
    let (_, v) = context_json(&sb, &["CHECKOUT"]);
    assert_eq!(v["resolved_targets"][0]["kind"], "feature");

    // Varios objetivos y --query.
    let (_, v) = context_json(
        &sb,
        &["module:auth", "feature:checkout", "--query", "centavos"],
    );
    let kinds: Vec<&str> = v["resolved_targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["module", "feature", "query"]);

    // Secretos: nunca aparecen aunque una nota los cite.
    let (_, v) = context_json(&sb, &["src/billing/", "--content", "full"]);
    assert!(!v.to_string().contains("credentials.json"), "{v:#}");
    assert!(!v.to_string().contains("s3cr3t"));
    let (code, v) = context_json(&sb, &["src/billing/credentials.json"]);
    assert_eq!(code, 2);
    assert_eq!(v["unresolved_targets"][0], "src/billing/credentials.json");
}

#[test]
fn context_unresolved_ambiguous_and_exit_codes() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    // Inexistente: código 2, JSON válido y sugerencia.
    let (code, v) = context_json(&sb, &["module:biling"]);
    assert_eq!(code, 2);
    assert!(v["resolved_targets"].as_array().unwrap().is_empty());
    assert!(
        v["warnings"].to_string().contains("module:billing"),
        "{v:#}"
    );
    // Uno resuelto y otro no: éxito con la lista de no resueltos.
    let (code, v) = context_json(&sb, &["module:auth", "nada-por-aqui"]);
    assert_eq!(code, 0);
    assert_eq!(v["unresolved_targets"][0], "nada-por-aqui");
    // Ambiguo: todas las coincidencias y aviso.
    fs::write(
        sb.root.join("knowledge/features/checkout2.md"),
        "---\nid: feat-checkout2\ntype: feature\ntitle: Checkout\nsources: [src/auth/login.py:1]\n---\n# Checkout 2\n",
    )
    .unwrap();
    // Mismo título: por id/archivo gana la coincidencia exacta; por título, ambas.
    let (code, v) = context_json(&sb, &["checkout"]);
    assert_eq!(code, 0);
    assert_eq!(
        v["resolved_targets"][0]["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (_, v) = context_json(&sb, &["feature:Checkout"]);
    assert_eq!(
        v["resolved_targets"][0]["matches"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(v["warnings"].to_string().contains("ambiguo"), "{v:#}");

    // Errores de uso: código 1 (no el 2 de clap).
    let (code, _, err) = context_run(&sb, "es", &["module:auth", "--content", "todo"]);
    assert_eq!(code, 1, "{err}");
    let (code, _, err) = context_run(&sb, "es", &["module:auth", "--json", "--format", "text"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("--json"), "{err}");
    let (code, _, _) = context_run(&sb, "es", &[]);
    assert_eq!(code, 1);
    let (code, _, _) = context_run(&sb, "es", &["module:auth", "--include", "modulos"]);
    assert_eq!(code, 1);
    // `--json` es alias de `--format json`.
    let (code, out, _) = context_run(&sb, "es", &["module:auth", "--json"]);
    assert_eq!(code, 0);
    assert_context_schema(&serde_json::from_str(&out).unwrap());
    // Sin proyecto: código 1.
    let empty = Sandbox::new();
    let (code, _, _) = context_run(&empty, "es", &["src"]);
    assert_eq!(code, 1);
    // Los demás comandos conservan el código de uso de clap.
    let out = sb
        .cmd()
        .args(["status", "--format", "xml"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn context_depth_and_dependencies() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let (_, v0) = context_json(&sb, &["feature:checkout", "--depth", "0"]);
    assert!(v0["dependencies"]["outgoing"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!str_list(&v0["notes"], "path").contains(&"knowledge/modules/auth.md".to_string()));
    let (_, v1) = context_json(&sb, &["feature:checkout", "--depth", "1"]);
    let out = v1["dependencies"]["outgoing"].as_array().unwrap();
    assert!(
        out.iter()
            .any(|e| e["to"] == "module:auth" && e["depth"] == 1),
        "{v1:#}"
    );
    let auth = v1["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == "knowledge/modules/auth.md")
        .unwrap();
    assert_eq!(auth["relevance"], 0.6);
    let (_, v2) = context_json(&sb, &["module:auth", "--depth", "2"]);
    let incoming = v2["dependencies"]["incoming"].as_array().unwrap();
    assert!(
        incoming
            .iter()
            .any(|e| e["from"] == "feature:checkout" && e["depth"] == 1),
        "{v2:#}"
    );
    assert!(incoming.iter().any(|e| e["depth"] == 2), "{v2:#}");
}

#[test]
fn context_stale_and_fail_on_stale() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let (code, _) = context_json(&sb, &["module:auth", "--fail-on-stale"]);
    assert_eq!(code, 0);
    // Pendiente: la fuente cambió desde el último sync/scan.
    sb.run(&["sync"]);
    fs::write(sb.root.join("src/auth/login.py"), "def login(): return 1\n").unwrap();
    let (code, v) = context_json(&sb, &["module:auth", "--fail-on-stale"]);
    assert_eq!(code, 3, "{v:#}");
    assert!(v["notes"][0]["stale_reason"]
        .as_str()
        .unwrap()
        .contains("adoc sync"));
    let (code, _) = context_json(&sb, &["module:auth"]);
    assert_eq!(code, 0);
    // Marcada por `sync`: mismo criterio.
    sb.run(&["sync"]);
    let (code, v) = context_json(&sb, &["module:auth", "--fail-on-stale"]);
    assert_eq!(code, 3);
    assert_eq!(v["modules"][0]["stale"], true);
    assert!(v["warnings"][0]
        .as_str()
        .unwrap()
        .contains("desactualizada"));
}

#[test]
fn context_limits_and_content_modes() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let (_, v) = context_json(&sb, &["src/billing/", "--max-notes", "2"]);
    assert_eq!(v["notes"].as_array().unwrap().len(), 2);
    assert!(v["warnings"].to_string().contains("--max-notes"));
    let rel: Vec<f64> = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["relevance"].as_f64().unwrap())
        .collect();
    assert!(rel[0] >= rel[1]);

    let (_, v) = context_json(&sb, &["src/billing/", "--max-chars", "30"]);
    let total: usize = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            n["content"]
                .as_str()
                .unwrap()
                .replace("…[truncated]", "")
                .chars()
                .count()
        })
        .sum();
    assert!(total <= 30, "{total}");
    assert!(v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["truncated"] == true));
    assert!(v["warnings"].to_string().contains("--max-chars"));

    let (_, none) = context_json(&sb, &["feature:checkout", "--content", "none"]);
    assert_eq!(none["notes"][0]["content"], "");
    let (_, ex) = context_json(&sb, &["feature:checkout"]);
    assert_eq!(
        ex["notes"][0]["content"],
        "Cobra el carrito y emite la factura."
    );
    let (_, full) = context_json(&sb, &["feature:checkout", "--content", "full"]);
    let content = full["notes"][0]["content"].as_str().unwrap();
    assert!(content.contains("Requiere sesión") && !content.contains("archeology-doc:begin"));

    // --include: las secciones excluidas quedan como listas vacías.
    let (_, v) = context_json(&sb, &["src/billing/", "--include", "findings,tests"]);
    assert!(v["notes"].as_array().unwrap().is_empty());
    assert!(v["modules"].as_array().unwrap().is_empty());
    assert!(!v["findings"].as_array().unwrap().is_empty());
    // --all-findings: también los resueltos.
    let (_, v) = context_json(&sb, &["src/billing/", "--all-findings"]);
    assert_eq!(v["findings"].as_array().unwrap().len(), 2);
    // Salida determinista (salvo la fecha).
    let strip = |mut v: serde_json::Value| {
        v["project"]["generated_at"] = serde_json::Value::Null;
        v
    };
    let (_, a) = context_json(&sb, &["src/billing/"]);
    let (_, b) = context_json(&sb, &["src/billing/"]);
    assert_eq!(strip(a), strip(b));
}

#[test]
fn context_text_and_markdown_follow_the_interface_language() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let (code, es, _) = context_run(&sb, "es", &["src/billing/"]);
    assert_eq!(code, 0);
    for s in [
        "Zona",
        "Módulos",
        "Hallazgos abiertos",
        "Tests",
        "Siguiente paso",
        "falta: Tarjeta rechazada",
    ] {
        assert!(es.contains(s), "{s}\n{es}");
    }
    let (_, en, _) = context_run(&sb, "en", &["src/billing/"]);
    for s in [
        "Zone",
        "Modules",
        "Open findings",
        "Next step",
        "missing: Tarjeta rechazada",
    ] {
        assert!(en.contains(s), "{s}\n{en}");
    }
    assert!(!en.contains('\x1b'), "sin colores fuera de una terminal");
    let (_, md, _) = context_run(&sb, "en", &["src/billing/", "--format", "markdown"]);
    assert!(md.starts_with("# Context: `src/billing/`"), "{md}");
    assert!(md.contains("## Open findings"));
    assert!(
        md.contains("`src/billing/invoice.py:4-99` (invalid evidence)"),
        "{md}"
    );
    let (code, out, _) = context_run(&sb, "en", &["module:nope"]);
    assert_eq!(code, 2);
    assert!(out.contains("No target could be resolved"), "{out}");
    // doctor anuncia la versión del contrato.
    let doctor: serde_json::Value =
        serde_json::from_str(&sb.run(&["doctor", "--format", "json"])).unwrap();
    assert_eq!(doctor["context_schema_version"], 1);
}

/// `git add -A && git commit` en el sandbox.
fn context_git_commit(root: &Path, msg: &str) {
    for args in [
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            msg,
        ],
    ] {
        let ok = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
}

fn context_state(sb: &Sandbox) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(sb.work().join("state.json")).unwrap()).unwrap()
}

/// `scan` no fija el commit documentado (solo `sync`) ni necesita el estado;
/// el aviso de commit solo aparece si cambió código, no por commitear el vault.
#[test]
fn context_commit_warning_only_for_source_changes() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    git_init(&sb.root);
    sb.run(&["scan"]);
    let state_path = sb.work().join("state.json");
    if state_path.is_file() {
        assert!(context_state(&sb).get("knowledge_commit").is_none());
    }
    sb.run(&["sync"]);
    assert!(context_state(&sb)["knowledge_commit"].is_string());
    let commit_warning = |v: &serde_json::Value| {
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("HEAD"))
    };

    // Commitear solo el vault y los archivos de adoc: sin aviso.
    let note = sb.root.join("knowledge/modules/auth.md");
    let text = fs::read_to_string(&note).unwrap();
    fs::write(&note, format!("{text}\nMás detalle.\n")).unwrap();
    fs::write(sb.root.join("AGENTS.md"), "# agentes\n").unwrap();
    context_git_commit(&sb.root, "docs");
    let (_, v) = context_json(&sb, &["module:auth"]);
    assert!(!commit_warning(&v), "{v:#}");

    // Un cambio de código commiteado: aviso con el número de archivos.
    fs::write(sb.root.join("src/auth/login.py"), "def login(): return 3\n").unwrap();
    context_git_commit(&sb.root, "code");
    let (_, v) = context_json(&sb, &["module:auth"]);
    assert!(
        v["warnings"].to_string().contains("1 archivo(s) de código"),
        "{v:#}"
    );

    // Commit documentado desconocido: aviso más suave.
    let mut st = context_state(&sb);
    st["knowledge_commit"] = "0123456789abcdef0123456789abcdef01234567".into();
    fs::write(sb.work().join("state.json"), st.to_string()).unwrap();
    let (_, v) = context_json(&sb, &["module:auth"]);
    assert!(
        v["warnings"]
            .to_string()
            .contains("no está en el historial"),
        "{v:#}"
    );

    // `scan` con un state.json corrupto no se interrumpe.
    fs::write(sb.work().join("state.json"), "{corrupto").unwrap();
    fs::remove_file(sb.root.join("knowledge/00-overview.md")).unwrap();
    sb.run(&["scan"]);
    assert!(sb.root.join("knowledge/00-overview.md").is_file());
}

/// `../billing` desde `src/auth` y la raíz absoluta del proyecto.
#[test]
fn context_cwd_relative_and_absolute_root_targets() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let out = sb
        .cmd()
        .current_dir(sb.root.join("src/auth"))
        .args(["context", "../billing", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_context_schema(&v);
    assert_eq!(v["resolved_targets"][0]["kind"], "path");
    assert!(str_list(&v["notes"], "path").contains(&"knowledge/modules/billing.md".to_string()));
    let root = sb.root.to_string_lossy().to_string();
    for target in [root.clone(), format!("{root}/")] {
        let (code, v) = context_json(&sb, &[&target]);
        assert_eq!(code, 0, "{target}");
        assert_eq!(v["resolved_targets"][0]["kind"], "path");
        assert!(!v["notes"].as_array().unwrap().is_empty());
    }
}

/// Solo `--query`: las coincidencias hacen de zona (funcionalidades, tests,
/// dependencias) y `--depth` aplica.
#[test]
fn context_query_only_populates_the_zone() {
    let sb = Sandbox::new();
    context_fixture(&sb);
    let (code, v) = context_json(&sb, &["--query", "carrito"]);
    assert_eq!(code, 0);
    assert_eq!(v["resolved_targets"][0]["kind"], "query");
    assert_eq!(v["features"][0]["id"], "feature:checkout", "{v:#}");
    assert_eq!(v["modules"][0]["id"], "module:billing");
    assert!(!v["tests"]["missing"].as_array().unwrap().is_empty());
    assert!(!v["dependencies"]["outgoing"].as_array().unwrap().is_empty());
    let (_, v0) = context_json(&sb, &["--query", "carrito", "--depth", "0"]);
    assert!(v0["dependencies"]["outgoing"]
        .as_array()
        .unwrap()
        .is_empty());
}
