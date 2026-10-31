//! `adoc context`: el contexto documentado de una o varias zonas del código
//! en una sola llamada, para agentes y scripts (contrato versionado en
//! `schemas/context.v1.schema.json`, documentado en `docs/context.md`).
//!
//! Es de solo lectura y no usa agentes ni red: notas del vault (leídas una
//! vez con [`load_vault`]), el grafo ([`build_from`]), `state.json`,
//! `inventory.json`, las tareas y `git rev-parse HEAD`.
//!
//! Criterios compartidos con otros comandos:
//! - `stale`: el de `adoc sync` (`status: stale` en la nota) más el
//!   "pendiente": una fuente citada cuyo hash ya no coincide con el guardado
//!   en el último `sync`/`scan` (lo que `sync` marcaría).
//! - `evidence[].valid`: el de `adoc check` ([`evidence_status`]).
//! - Secretos: nunca se listan ni se leen evidencias que coinciden con
//!   `scan.secrets_patterns`.

use crate::frontmatter::{
    evidence_status, extract_frontmatter, looks_like_secret_path, EvidenceStatus, NoteFrontmatter,
};
use crate::graph::build_from;
use crate::index::{
    load_vault, normalize_rel_path, normalize_source, source_file, source_lines, VaultNote,
};
use crate::model::Inventory;
use crate::project::Project;
use crate::state::State;
use crate::t;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Versión del contrato JSON (`schema_version`). Campos opcionales nuevos no
/// la incrementan; cambiar o eliminar campos sí.
pub const SCHEMA_VERSION: u32 = 1;

/// Marcador que se añade al contenido recortado por `--max-chars`.
pub const TRUNCATION_MARK: &str = "…[truncated]";

/// Longitud máxima de los resúmenes (`summary`, `definition`).
const SUMMARY_CHARS: usize = 300;
/// Longitud orientativa de `--content excerpt`.
const EXCERPT_CHARS: usize = 600;
/// Sugerencias que se muestran cuando un objetivo no se resuelve.
const MAX_SUGGESTIONS: usize = 3;

// ---------------------------------------------------------------------------
// Opciones
// ---------------------------------------------------------------------------

/// Secciones de `--include`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Modules,
    Features,
    Notes,
    Findings,
    Tests,
    Dependencies,
    Decisions,
    Glossary,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Modules,
        Section::Features,
        Section::Notes,
        Section::Findings,
        Section::Tests,
        Section::Dependencies,
        Section::Decisions,
        Section::Glossary,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Section::Modules => "modules",
            Section::Features => "features",
            Section::Notes => "notes",
            Section::Findings => "findings",
            Section::Tests => "tests",
            Section::Dependencies => "dependencies",
            Section::Decisions => "decisions",
            Section::Glossary => "glossary",
        }
    }

    pub fn parse(value: &str) -> Option<Section> {
        Section::ALL
            .into_iter()
            .find(|s| s.as_str() == value.trim().to_ascii_lowercase())
    }
}

/// Contenido de las notas (`--content`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    /// Sin contenido (`content: ""`).
    None,
    /// Resumen (región `summary`) o primeros párrafos.
    Excerpt,
    /// Cuerpo completo sin frontmatter ni marcadores de regiones.
    Full,
}

impl ContentMode {
    pub fn parse(value: &str) -> Option<ContentMode> {
        match value {
            "none" => Some(ContentMode::None),
            "excerpt" => Some(ContentMode::Excerpt),
            "full" => Some(ContentMode::Full),
            _ => None,
        }
    }
}

/// Parámetros de [`build`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Objetivos tal como los escribió el usuario.
    pub targets: Vec<String>,
    /// `--query`: búsqueda en títulos y contenido.
    pub query: Option<String>,
    /// Saltos del grafo para dependencias y notas relacionadas.
    pub depth: usize,
    pub include: BTreeSet<Section>,
    /// Máximo de notas en `notes` (por relevancia).
    pub max_notes: usize,
    /// Presupuesto total de caracteres del contenido de las notas (0 = sin límite).
    pub max_chars: usize,
    pub content: ContentMode,
    /// Incluir hallazgos resueltos y descartados.
    pub all_findings: bool,
    /// Directorio actual (rutas relativas a él si está dentro del proyecto).
    pub cwd: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            query: None,
            depth: 1,
            include: Section::ALL.into_iter().collect(),
            max_notes: 20,
            max_chars: 0,
            content: ContentMode::Excerpt,
            all_findings: false,
            cwd: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Contrato JSON (schema_version 1)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Context {
    pub schema_version: u32,
    pub adoc_version: String,
    pub project: ProjectRef,
    pub resolved_targets: Vec<ResolvedTarget>,
    pub unresolved_targets: Vec<String>,
    pub modules: Vec<ModuleCtx>,
    pub features: Vec<FeatureCtx>,
    pub notes: Vec<NoteCtx>,
    pub findings: Vec<FindingCtx>,
    pub tests: TestsCtx,
    pub dependencies: DependenciesCtx,
    pub decisions: Vec<DecisionCtx>,
    pub glossary: Vec<GlossaryCtx>,
    pub open_questions: Vec<QuestionCtx>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub root: String,
    pub knowledge_commit: Option<String>,
    pub head_commit: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ResolvedTarget {
    pub input: String,
    /// `path`, `module`, `feature` o `query`.
    pub kind: String,
    pub matches: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModuleCtx {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub paths: Vec<String>,
    pub note: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Serialize)]
pub struct FeatureCtx {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub modules: Vec<String>,
    pub note: String,
    pub stale: bool,
}

#[derive(Debug, Serialize)]
pub struct NoteCtx {
    pub path: String,
    pub title: String,
    /// Tipo de nota (`module`, `feature`, `finding`…).
    #[serde(rename = "type")]
    pub note_type: String,
    pub relevance: f64,
    pub stale: bool,
    pub stale_reason: Option<String>,
    pub content: String,
    pub truncated: bool,
    pub evidence: Vec<EvidenceCtx>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceCtx {
    pub file: String,
    pub line: Option<usize>,
    pub end_line: Option<usize>,
    pub valid: bool,
}

#[derive(Debug, Serialize)]
pub struct FindingCtx {
    pub id: String,
    pub severity: String,
    pub category: String,
    /// Valor original de `kind` en la nota.
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub evidence: Vec<EvidenceCtx>,
    pub note: String,
    pub stale: bool,
}

#[derive(Debug, Serialize, Default)]
pub struct TestsCtx {
    pub covered: Vec<CoveredScenario>,
    pub missing: Vec<MissingScenario>,
    pub disabled_or_empty: Vec<DisabledTest>,
}

#[derive(Debug, Serialize)]
pub struct CoveredScenario {
    pub feature: String,
    pub scenario: String,
    pub tests: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MissingScenario {
    pub feature: String,
    pub scenario: String,
}

#[derive(Debug, Serialize)]
pub struct DisabledTest {
    pub file: String,
    pub line: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct DependenciesCtx {
    pub outgoing: Vec<DependencyEdge>,
    pub incoming: Vec<DependencyEdge>,
}

#[derive(Debug, Serialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub depth: usize,
    /// Tipos de arista del grafo (`link`, `module`, `related`, `tests`).
    pub kinds: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DecisionCtx {
    pub title: String,
    pub note: String,
    pub summary: String,
}

#[derive(Debug, Serialize)]
pub struct GlossaryCtx {
    pub term: String,
    pub definition: String,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct QuestionCtx {
    /// Id de la tarea (`T-014`) o `memory#N` (dudas para el equipo).
    pub id: String,
    pub question: String,
    pub related: Vec<String>,
}

impl Context {
    /// ¿Se resolvió al menos un objetivo (o la búsqueda)?
    pub fn resolved_any(&self) -> bool {
        !self.resolved_targets.is_empty()
    }

    /// Notas desactualizadas (marcadas o pendientes de `sync`) en la salida.
    pub fn stale_count(&self) -> usize {
        let mut paths: BTreeSet<&str> = BTreeSet::new();
        paths.extend(
            self.notes
                .iter()
                .filter(|n| n.stale)
                .map(|n| n.path.as_str()),
        );
        paths.extend(
            self.modules
                .iter()
                .filter(|m| m.stale)
                .filter_map(|m| m.note.as_deref()),
        );
        paths.extend(
            self.features
                .iter()
                .filter(|f| f.stale)
                .map(|f| f.note.as_str()),
        );
        paths.extend(
            self.findings
                .iter()
                .filter(|f| f.stale)
                .map(|f| f.note.as_str()),
        );
        paths.len()
    }
}

// ---------------------------------------------------------------------------
// Utilidades de texto y rutas
// ---------------------------------------------------------------------------

/// Minúsculas y sin acentos: comparación de títulos y búsquedas.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// ¿`path` está dentro de `base` (o es `base`)? `base` vacío = raíz.
pub fn within(path: &str, base: &str) -> bool {
    base.is_empty()
        || path == base
        || (path.len() > base.len()
            && path.starts_with(base)
            && path[base.len()..].starts_with('/'))
}

/// Directorio padre de una ruta relativa (`""` en la raíz).
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

/// Ruta de módulo del inventario normalizada (`.` → `""`, sin `/` final).
fn module_path(path: &str) -> String {
    let p = normalize_rel_path(path);
    if p == "." {
        String::new()
    } else {
        p.trim_end_matches('/').to_string()
    }
}

/// ¿`word` aparece en `text` como palabra completa (ambos ya en [`fold`])?
fn mentions_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut start = 0;
    while let Some(pos) = text[start..].find(word) {
        let i = start + pos;
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            return true;
        }
        start = i + word.len().max(1);
    }
    false
}

/// Recorta a `max` caracteres con `…` (sin partir caracteres).
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

/// Distancia de edición (sugerencias para objetivos sin coincidencia).
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Cuerpo de la nota sin frontmatter.
fn body(text: &str) -> &str {
    extract_frontmatter(text).map(|(_, b)| b).unwrap_or(text)
}

/// Contenido de la región gestionada `name`, si existe y no es plantilla.
fn region_content(text: &str, name: &str) -> Option<String> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if crate::managed::is_marker(t, &format!("begin {name} ")) {
            inside = true;
            continue;
        }
        if inside && crate::managed::is_marker(t, &format!("end {name} ")) {
            break;
        }
        if inside {
            out.push(line);
        }
    }
    let content = out.join("\n").trim().to_string();
    (!content.is_empty() && !content.contains("{{")).then_some(content)
}

/// Párrafos de texto del cuerpo: sin regiones gestionadas (salvo que se
/// pidan aparte), encabezados, citas ni comentarios HTML.
fn paragraphs(body: &str) -> Vec<String> {
    let clean = crate::managed::strip_regions(body).unwrap_or_else(|| body.to_string());
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let flush = |current: &mut Vec<&str>, out: &mut Vec<String>| {
        if !current.is_empty() {
            out.push(current.join("\n"));
            current.clear();
        }
    };
    for line in clean.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            flush(&mut current, &mut out);
            continue;
        }
        if t.starts_with('>') || t.starts_with("<!--") || t.contains("{{") {
            continue;
        }
        current.push(line.trim_end());
    }
    flush(&mut current, &mut out);
    out
}

/// Primer párrafo de la sección `## <nombre>` (cualquiera de `names`, en [`fold`]).
fn section_paragraph(body: &str, names: &[&str]) -> Option<String> {
    let mut lines = body.lines();
    for line in lines.by_ref() {
        if let Some(h) = line.trim().strip_prefix("## ") {
            if names.contains(&fold(h.trim()).as_str()) {
                break;
            }
        }
    }
    let rest: Vec<&str> = lines
        .take_while(|l| !l.trim_start().starts_with('#'))
        .collect();
    paragraphs(&rest.join("\n")).into_iter().next()
}

/// Resumen de una nota: región `summary` o primer párrafo.
fn summary_of(text: &str) -> String {
    let b = body(text);
    let s = region_content(b, "summary")
        .or_else(|| paragraphs(b).into_iter().next())
        .unwrap_or_default();
    clip(
        &s.split_whitespace().collect::<Vec<_>>().join(" "),
        SUMMARY_CHARS,
    )
}

/// Contenido de una nota según `--content`.
fn content_of(text: &str, mode: ContentMode) -> String {
    let b = body(text);
    match mode {
        ContentMode::None => String::new(),
        ContentMode::Full => b
            .lines()
            .filter(|l| {
                let t = l.trim();
                !(crate::managed::is_marker(t, "begin") || crate::managed::is_marker(t, "end"))
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        ContentMode::Excerpt => {
            if let Some(s) = region_content(b, "summary") {
                return s;
            }
            let mut out = String::new();
            for (i, p) in paragraphs(b).into_iter().enumerate() {
                if i > 0 && out.chars().count() >= EXCERPT_CHARS / 2 {
                    break;
                }
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&p);
                if i >= 1 {
                    break;
                }
            }
            out
        }
    }
}

/// Categoría del contrato a partir del `kind` de un hallazgo.
pub fn finding_category(kind: &str) -> &'static str {
    match kind {
        "contradiction" => "contradiction",
        "stale-doc" => "outdated_doc",
        "duplication" => "duplicated_rule",
        "config" => "config_conflict",
        "dead-code" => "dead_code",
        _ => "other",
    }
}

/// Estado del contrato a partir del `state` de un hallazgo.
pub fn finding_status(state: &str) -> &'static str {
    match state {
        "resolved" => "resolved",
        "dismissed" => "dismissed",
        // `open`, `confirmed` o sin valor: sigue abierto.
        _ => "open",
    }
}

fn severity_rank(sev: &str) -> usize {
    match sev {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

/// Escenarios de un reporte de tests (secciones en inglés o español, en
/// lista o tabla): (cubiertos con sus referencias a tests, faltantes).
/// Clasifica un encabezado (ya en [`fold`]): `Some(true)` = cubiertos,
/// `Some(false)` = faltantes. Las negaciones se comprueban primero:
/// "Uncovered", "Not covered", "No cubiertos" o "Sin cubrir" son faltantes.
fn scenario_heading(h: &str) -> Option<bool> {
    let words: Vec<&str> = h
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let follows = |a: &str, bs: &[&str]| words.windows(2).any(|p| p[0] == a && bs.contains(&p[1]));
    let missing = has("uncovered")
        || has("missing")
        || has("faltante")
        || has("faltantes")
        || follows("not", &["covered"])
        || follows("no", &["cubierto", "cubiertos", "covered"])
        || follows("sin", &["cubrir", "cobertura", "tests"]);
    if missing {
        return Some(false);
    }
    if has("covered") || has("cubierto") || has("cubiertos") {
        return Some(true);
    }
    None
}

pub fn parse_scenarios(body: &str) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    #[derive(PartialEq)]
    enum Part {
        None,
        Covered,
        Missing,
    }
    let mut part = Part::None;
    let mut table_header_seen = false;
    let mut covered = Vec::new();
    let mut missing = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix('#') {
            part = match scenario_heading(&fold(h.trim_start_matches('#'))) {
                Some(true) => Part::Covered,
                Some(false) => Part::Missing,
                None => Part::None,
            };
            table_header_seen = false;
            continue;
        }
        if part == Part::None || t.is_empty() {
            continue;
        }
        let (scenario, tests) = if t.starts_with('|') {
            let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
            if cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
            {
                continue;
            }
            if !table_header_seen {
                // Primera fila de la tabla: encabezados.
                table_header_seen = true;
                continue;
            }
            let scenario = cells.first().copied().unwrap_or("").to_string();
            let tests = cells
                .get(1)
                .map(|c| test_refs(c))
                .filter(|_| part == Part::Covered)
                .unwrap_or_default();
            (scenario, tests)
        } else if let Some(item) = list_item(t) {
            let refs = test_refs(item);
            let mut scenario = item.to_string();
            for r in &refs {
                scenario = scenario.replace(r.as_str(), "");
            }
            let scenario = scenario
                .replace('`', "")
                .trim()
                .trim_end_matches(['—', '-', ':', '(', ')', ',', ' '])
                .trim()
                .to_string();
            (scenario, refs)
        } else {
            continue;
        };
        if scenario.is_empty() {
            continue;
        }
        match part {
            Part::Covered => covered.push((scenario, tests)),
            Part::Missing => missing.push(scenario),
            Part::None => {}
        }
    }
    (covered, missing)
}

/// Texto de un elemento de lista (`- x`, `* x`, `1. x`).
fn list_item(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(rest.trim());
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        return line[digits..]
            .strip_prefix(". ")
            .or_else(|| line[digits..].strip_prefix(") "))
            .map(str::trim);
    }
    None
}

/// Referencias a archivos de test en un texto (`tests/a_test.rs:12`).
fn test_refs(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '`' | '(' | ')' | '[' | ']'))
        .map(|t| t.trim_end_matches(['.', ':']))
        .filter(|t| {
            let file = source_file(t);
            let has_ext = file
                .rsplit_once('.')
                .is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.len() <= 6);
            has_ext && (file.contains('/') || source_lines(t).is_some())
        })
        .map(normalize_rel_path)
        .collect()
}

// ---------------------------------------------------------------------------
// Catálogo del vault
// ---------------------------------------------------------------------------

/// Módulo: del inventario (con o sin nota) o solo nota `type: module`.
struct ModuleEntry {
    name: String,
    /// Rutas normalizadas (`""` = raíz).
    paths: Vec<String>,
    recursive: bool,
    note: Option<usize>,
}

impl ModuleEntry {
    /// ¿El módulo y la ruta `p` se solapan (la ruta está en el módulo o el
    /// módulo está dentro de la carpeta)?
    fn touches(&self, p: &str) -> bool {
        self.paths.iter().any(|mp| {
            within(mp, p)
                || (self.recursive && within(p, mp))
                || (!self.recursive && (p == mp || parent(p) == mp))
        })
    }

    /// ¿El archivo `file` pertenece al módulo?
    fn contains_file(&self, file: &str) -> bool {
        self.paths.iter().any(|mp| {
            if self.recursive {
                within(file, mp)
            } else {
                parent(file) == mp
            }
        })
    }
}

/// Evidencia de una nota ya normalizada.
struct Source {
    raw: String,
    file: String,
}

struct Catalog<'a> {
    project: &'a Project,
    vault: &'a [VaultNote],
    by_id: HashMap<&'a str, usize>,
    sources: Vec<Vec<Source>>,
    modules: Vec<ModuleEntry>,
    module_of_note: HashMap<usize, usize>,
}

impl<'a> Catalog<'a> {
    fn new(project: &'a Project, vault: &'a [VaultNote], inv: Option<&Inventory>) -> Self {
        let by_id = vault
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let sources = vault
            .iter()
            .map(|n| {
                n.fm.as_ref()
                    .map(|fm| {
                        fm.sources
                            .iter()
                            .map(|raw| Source {
                                raw: raw.clone(),
                                file: normalize_source(&project.root, source_file(raw)),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let mut cat = Catalog {
            project,
            vault,
            by_id,
            sources,
            modules: Vec::new(),
            module_of_note: HashMap::new(),
        };
        let module_notes: Vec<usize> = (0..vault.len())
            .filter(|&i| vault[i].kind() == "module")
            .collect();
        let mut used: BTreeSet<usize> = BTreeSet::new();
        for m in inv.map(|i| i.modules.as_slice()).unwrap_or_default() {
            let note = cat.module_note_for(&m.name, &module_notes, &used);
            if let Some(n) = note {
                used.insert(n);
            }
            cat.modules.push(ModuleEntry {
                name: m.name.clone(),
                paths: vec![module_path(&m.path)],
                recursive: m.recursive,
                note,
            });
        }
        // Notas de módulo sin módulo en el inventario: sus rutas son las
        // carpetas de sus evidencias.
        for &n in &module_notes {
            if !used.contains(&n) {
                let mut paths: Vec<String> = cat.sources[n]
                    .iter()
                    .filter(|s| !s.file.starts_with('/'))
                    .map(|s| parent(&s.file).to_string())
                    // Archivos de la raíz: no convierten al módulo en todo el proyecto.
                    .filter(|p| !p.is_empty())
                    .collect();
                paths.sort();
                paths.dedup();
                cat.modules.push(ModuleEntry {
                    name: vault[n].stem().to_string(),
                    paths,
                    recursive: true,
                    note: Some(n),
                });
            }
        }
        cat.module_of_note = cat
            .modules
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.note.map(|n| (n, i)))
            .collect();
        cat
    }

    /// Nota de un módulo del inventario: `modules/<nombre>`, nombre de
    /// archivo, `id` (`<nombre>` o `mod-<nombre>`) o título.
    fn module_note_for(
        &self,
        name: &str,
        notes: &[usize],
        used: &BTreeSet<usize>,
    ) -> Option<usize> {
        let free = |i: &&usize| !used.contains(*i);
        let fm_id = |i: usize| self.fm(i).and_then(|f| f.id.clone()).unwrap_or_default();
        let by =
            |pred: &dyn Fn(usize) -> bool| notes.iter().filter(free).copied().find(|&i| pred(i));
        by(&|i| self.vault[i].id == format!("modules/{name}"))
            .or_else(|| by(&|i| self.vault[i].stem() == name))
            .or_else(|| by(&|i| fm_id(i) == name || fm_id(i) == format!("mod-{name}")))
            .or_else(|| by(&|i| fold(&self.title(i)) == fold(name)))
    }

    fn fm(&self, i: usize) -> Option<&NoteFrontmatter> {
        self.vault[i].fm.as_ref()
    }

    fn title(&self, i: usize) -> String {
        self.fm(i)
            .and_then(|f| f.title.clone())
            .unwrap_or_else(|| self.vault[i].stem().to_string())
    }

    fn field(&self, i: usize, key: &str) -> Option<String> {
        self.fm(i)
            .and_then(|f| f.get(key))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    fn list(&self, i: usize, key: &str) -> Vec<String> {
        self.fm(i)
            .and_then(|f| f.get(key))
            .and_then(|v| v.as_sequence())
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Índices (MOC, `index.md`): enlazan a todo y no aportan contexto.
    fn is_index(&self, i: usize) -> bool {
        let stem = self.vault[i].stem();
        self.vault[i].kind() == "moc" || stem == "index" || stem == "_index"
    }

    /// Id del contrato: `module:<nombre>`, `feature:<archivo>` o la ruta.
    fn ref_id(&self, i: usize) -> String {
        if let Some(&m) = self.module_of_note.get(&i) {
            return format!("module:{}", self.modules[m].name);
        }
        if self.vault[i].kind() == "feature" {
            return format!("feature:{}", self.vault[i].stem());
        }
        self.vault[i].rel.clone()
    }

    /// Módulos (índices) del campo `modules:` de una nota, resueltos con los
    /// mismos alias que `module:` (`mod-billing`, id, título).
    fn feature_module_indexes(&self, i: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for raw in self.list(i, "modules") {
            if let Some(&m) = self.find_modules(raw.trim()).first() {
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Nombres canónicos del campo `modules:`; el valor original si no
    /// corresponde a ningún módulo conocido.
    fn feature_modules(&self, i: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for raw in self.list(i, "modules") {
            let name = match self.find_modules(raw.trim()).first() {
                Some(&m) => self.modules[m].name.clone(),
                None => raw.trim().to_string(),
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    /// Módulos por nombre: exacto, nota (`modules/x`, archivo, `id`) y título normalizado.
    fn find_modules(&self, name: &str) -> Vec<usize> {
        let exact: Vec<usize> = (0..self.modules.len())
            .filter(|&m| self.modules[m].name == name)
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let by_note: Vec<usize> = (0..self.modules.len())
            .filter(|&m| {
                self.modules[m].note.is_some_and(|n| {
                    let fm_id = self.fm(n).and_then(|f| f.id.clone()).unwrap_or_default();
                    self.vault[n].id == format!("modules/{name}")
                        || self.vault[n].stem() == name
                        || fm_id == name
                        || fm_id == format!("mod-{name}")
                })
            })
            .collect();
        if !by_note.is_empty() {
            return by_note;
        }
        let folded = fold(name);
        (0..self.modules.len())
            .filter(|&m| {
                fold(&self.modules[m].name) == folded
                    || self.modules[m]
                        .note
                        .is_some_and(|n| fold(&self.title(n)) == folded)
            })
            .collect()
    }

    /// Funcionalidades: `features/x`, archivo, `id` (`x`, `feat-x`) y título normalizado.
    fn find_features(&self, name: &str) -> Vec<usize> {
        let features: Vec<usize> = (0..self.vault.len())
            .filter(|&i| self.vault[i].kind() == "feature")
            .collect();
        let by_id: Vec<usize> = features
            .iter()
            .copied()
            .filter(|&i| {
                let fm_id = self.fm(i).and_then(|f| f.id.clone()).unwrap_or_default();
                self.vault[i].id == format!("features/{name}")
                    || self.vault[i].stem() == name
                    || fm_id == name
                    || fm_id == format!("feat-{name}")
            })
            .collect();
        if !by_id.is_empty() {
            return by_id;
        }
        let folded = fold(name);
        features
            .into_iter()
            .filter(|&i| fold(&self.title(i)) == folded)
            .collect()
    }

    /// Ruta del proyecto para un objetivo: relativa a la raíz o al directorio
    /// actual, existente o citada por alguna nota o módulo (archivo borrado).
    fn resolve_path(&self, input: &str, cwd: Option<&Path>) -> PathMatch {
        let root = &self.project.root;
        let raw = input.trim();
        let absolute = Path::new(raw).is_absolute()
            || raw.starts_with('/')
            || raw.starts_with('\\')
            || raw.get(1..3).is_some_and(|s| s == ":\\" || s == ":/");
        let mut candidates: Vec<String> = Vec::new();
        if absolute {
            // La raíz misma (`/proj`, `/proj/`) es todo el proyecto, como `.`.
            let text = normalize_rel_path(raw);
            let text = text.trim_end_matches('/');
            let is_root = [Some(root.clone()), crate::paths::canonicalize(root).ok()]
                .into_iter()
                .flatten()
                .any(|r| normalize_rel_path(&r.to_string_lossy()).trim_end_matches('/') == text);
            // Ruta existente a través de un symlink (`/var` → `/private/var`).
            let canonical = crate::paths::canonicalize(Path::new(raw))
                .ok()
                .zip(crate::paths::canonicalize(root).ok())
                .and_then(|(p, r)| {
                    p.strip_prefix(&r)
                        .ok()
                        .map(|rest| rest.to_string_lossy().replace('\\', "/"))
                });
            if is_root {
                candidates.push(String::new());
            } else if let Some(rel) = canonical {
                candidates.extend(lexical(&rel));
            } else {
                let rel = normalize_source(root, raw);
                if rel.starts_with('/') || rel.get(1..2) == Some(":") {
                    return PathMatch::None;
                }
                candidates.extend(lexical(&rel));
            }
        } else {
            let base = normalize_rel_path(raw);
            // Relativa a la raíz (sin salir de ella)…
            candidates.extend(lexical(&base));
            // …o al directorio actual: `../billing` desde `src/api`.
            if let Some(prefix) = cwd.and_then(|c| cwd_prefix(root, c)) {
                if !prefix.is_empty() {
                    if let Some(c) = lexical(&format!("{prefix}/{base}")) {
                        if !candidates.contains(&c) {
                            candidates.push(c);
                        }
                    }
                }
            }
        }
        for c in candidates {
            let full = if c.is_empty() {
                root.clone()
            } else {
                root.join(&c)
            };
            let exists = full.exists() && self.project.contains(&full);
            let cited = !c.is_empty()
                && (self.sources.iter().flatten().any(|s| within(&s.file, &c))
                    || self
                        .modules
                        .iter()
                        .any(|m| m.paths.iter().any(|mp| within(mp, &c))));
            if exists || cited {
                let secrets = &self.project.config.scan.secrets_patterns;
                if !c.is_empty() && looks_like_secret_path(&c, secrets) {
                    return PathMatch::Secret;
                }
                return PathMatch::Found(c);
            }
        }
        PathMatch::None
    }
}

enum PathMatch {
    Found(String),
    Secret,
    None,
}

/// Normaliza `.` y `..` de una ruta relativa; `None` si sale de la raíz.
/// Devuelve `""` para la raíz y nunca termina en `/`.
fn lexical(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Prefijo relativo del directorio actual dentro del proyecto (None si está fuera).
fn cwd_prefix(root: &Path, cwd: &Path) -> Option<String> {
    let root = crate::paths::canonicalize(root).ok()?;
    let cwd = crate::paths::canonicalize(cwd).ok()?;
    let rest = cwd.strip_prefix(&root).ok()?;
    Some(rest.to_string_lossy().replace('\\', "/"))
}

// ---------------------------------------------------------------------------
// Construcción
// ---------------------------------------------------------------------------

/// Zona resuelta a partir de los objetivos.
#[derive(Default)]
struct Zone {
    /// Rutas (archivos o carpetas) cuyas evidencias cuentan como "en la ruta".
    paths: Vec<String>,
    /// Módulos resueltos (índices de `Catalog::modules`) → relevancia directa.
    modules: BTreeMap<usize, f64>,
    /// Funcionalidades resueltas (índices del vault).
    features: BTreeSet<usize>,
    /// Notas que coinciden con `--query` (índice → coincide en el título).
    query: BTreeMap<usize, bool>,
}

/// Relevancia por nivel (bandas sin solaparse; ver `docs/context.md`).
fn tier_direct() -> f64 {
    1.0
}
fn tier_path_module() -> f64 {
    0.9
}
fn tier_evidence(count: usize) -> f64 {
    0.75 + 0.14 * (count.min(3) as f64 / 3.0)
}
fn tier_dependency(depth: usize) -> f64 {
    if depth <= 1 {
        0.6
    } else {
        0.3 / depth as f64
    }
}
fn tier_query(in_title: bool) -> f64 {
    if in_title {
        0.45
    } else {
        0.35
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Construye el contexto de los objetivos de `opts`.
pub fn build(project: &Project, opts: &Options) -> Context {
    let vault = load_vault(project);
    build_with(project, &vault, opts)
}

/// Como [`build`], con las notas ya leídas.
pub fn build_with(project: &Project, vault: &[VaultNote], opts: &Options) -> Context {
    let mut warnings: Vec<String> = Vec::new();
    let state = match State::load(&project.state_file()) {
        Ok(s) => s,
        Err(e) => {
            warnings.push(t!("context.warn.state_unreadable", error = e));
            State::default()
        }
    };
    let inventory: Option<Inventory> = std::fs::read_to_string(project.inventory_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    if inventory.is_none() {
        warnings.push(t!("context.warn.no_inventory"));
    }
    let cat = Catalog::new(project, vault, inventory.as_ref());
    let secrets = &project.config.scan.secrets_patterns;

    // --- Commits -----------------------------------------------------------
    let head_commit = crate::gitcmd::head_commit(&project.root);
    let knowledge_commit = state.knowledge_commit.clone().or_else(|| {
        inventory
            .as_ref()
            .and_then(|i| i.git.as_ref())
            .and_then(|g| g.last_commit.as_deref())
            .and_then(|c| c.split('|').next())
            .filter(|h| !h.is_empty())
            .map(String::from)
    });
    // Solo avisa si cambió código desde el commit documentado: commitear el
    // propio vault (lo normal tras `sync`) mueve HEAD sin desactualizar nada.
    if let (Some(k), Some(h)) = (&knowledge_commit, &head_commit) {
        if k != h {
            match changed_sources_between(project, k, h) {
                Some(files) if files.is_empty() => {}
                Some(files) => warnings.push(t!(
                    "context.warn.commit_mismatch",
                    count = files.len(),
                    knowledge = short(k),
                    head = short(h)
                )),
                None => warnings.push(t!(
                    "context.warn.commit_unknown",
                    knowledge = short(k),
                    head = short(h)
                )),
            }
        }
    }

    // --- Resolución de objetivos -----------------------------------------
    let mut zone = Zone::default();
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for input in &opts.targets {
        match resolve_target(&cat, input, opts.cwd.as_deref(), &mut zone, &mut warnings) {
            Some(r) => resolved.push(r),
            None => {
                unresolved.push(input.clone());
                let hint = suggestions(&cat, input);
                warnings.push(if hint.is_empty() {
                    t!("context.warn.unresolved", input = input)
                } else {
                    t!(
                        "context.warn.unresolved_suggest",
                        input = input,
                        suggestions = hint.join(", ")
                    )
                });
            }
        }
    }
    if let Some(q) = opts.query.as_deref().filter(|q| !q.trim().is_empty()) {
        let fq = fold(q.trim());
        for (i, n) in vault.iter().enumerate() {
            if cat.is_index(i) {
                continue;
            }
            let in_title = fold(&cat.title(i)).contains(&fq);
            if in_title || fold(body(&n.text)).contains(&fq) {
                zone.query.insert(i, in_title);
            }
        }
        if zone.query.is_empty() {
            unresolved.push(q.to_string());
            warnings.push(t!("context.warn.query_no_match", query = q));
        } else {
            let mut matches: Vec<String> = zone.query.keys().map(|&i| cat.ref_id(i)).collect();
            matches.sort();
            resolved.push(ResolvedTarget {
                input: q.to_string(),
                kind: "query".into(),
                matches,
            });
        }
    }

    // Solo `--query` (sin objetivos): las mejores coincidencias (hasta
    // `--max-notes`) hacen de zona, así `--depth`, módulos, funcionalidades,
    // tests y dependencias funcionan igual. Con objetivos, la búsqueda solo
    // añade notas.
    let mut query_seeds: BTreeSet<usize> = BTreeSet::new();
    let mut query_features: BTreeSet<usize> = BTreeSet::new();
    if opts.targets.is_empty() {
        let mut top: Vec<(usize, bool)> = zone.query.iter().map(|(&i, &t)| (i, t)).collect();
        top.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| vault[a.0].rel.cmp(&vault[b.0].rel))
        });
        for (i, in_title) in top.into_iter().take(opts.max_notes.max(1)) {
            query_seeds.insert(i);
            if let Some(&m) = cat.module_of_note.get(&i) {
                zone.modules.entry(m).or_insert(tier_query(in_title));
            }
            if vault[i].kind() == "feature" {
                query_features.insert(i);
            }
        }
    }

    // Rutas de los módulos y fuentes de las funcionalidades resueltas
    // (objetivo `module:`/`feature:`) también cuentan como "en la ruta".
    let mut evidence_paths: Vec<String> = zone.paths.clone();
    for (&m, &rel) in &zone.modules {
        if rel >= tier_direct() {
            evidence_paths.extend(cat.modules[m].paths.iter().cloned());
        }
    }
    for &f in &zone.features {
        evidence_paths.extend(cat.sources[f].iter().map(|s| s.file.clone()));
    }
    evidence_paths.sort();
    evidence_paths.dedup();

    // --- Relevancia ----------------------------------------------------------
    let mut score: BTreeMap<usize, f64> = BTreeMap::new();
    let bump = |score: &mut BTreeMap<usize, f64>, i: usize, s: f64| {
        let e = score.entry(i).or_insert(0.0);
        if s > *e {
            *e = s;
        }
    };
    for (&m, &rel) in &zone.modules {
        if let Some(n) = cat.modules[m].note {
            bump(&mut score, n, rel);
        }
    }
    for &f in &zone.features {
        bump(&mut score, f, tier_direct());
    }
    if !evidence_paths.is_empty() {
        for (i, srcs) in cat.sources.iter().enumerate() {
            let count = srcs
                .iter()
                .filter(|s| evidence_paths.iter().any(|p| within(&s.file, p)))
                .count();
            if count > 0 {
                bump(&mut score, i, tier_evidence(count));
            }
        }
    }
    let seeds: BTreeSet<usize> = score
        .keys()
        .copied()
        .chain(query_seeds.iter().copied())
        .filter(|&i| !cat.is_index(i))
        .collect();

    // Grafo (mismas aristas que `adoc graph`, sin evidencias ni índices).
    let graph = build_from(project, vault);
    let mut out_edges: BTreeMap<usize, BTreeMap<usize, BTreeSet<String>>> = BTreeMap::new();
    let mut in_edges: BTreeMap<usize, BTreeMap<usize, BTreeSet<String>>> = BTreeMap::new();
    for e in graph.edges.iter().filter(|e| e.kind != "evidence") {
        let (Some(&s), Some(&t)) = (
            cat.by_id.get(e.source.as_str()),
            cat.by_id.get(e.target.as_str()),
        ) else {
            continue;
        };
        if cat.is_index(s) || cat.is_index(t) {
            continue;
        }
        out_edges
            .entry(s)
            .or_default()
            .entry(t)
            .or_default()
            .insert(e.kind.clone());
        in_edges
            .entry(t)
            .or_default()
            .entry(s)
            .or_default()
            .insert(e.kind.clone());
    }
    // Distancia sin dirección desde la zona (notas relacionadas).
    let mut dist: BTreeMap<usize, usize> = seeds.iter().map(|&s| (s, 0)).collect();
    let mut queue: VecDeque<usize> = seeds.iter().copied().collect();
    while let Some(n) = queue.pop_front() {
        let d = dist[&n];
        if d >= opts.depth {
            continue;
        }
        let neighbors = out_edges
            .get(&n)
            .into_iter()
            .chain(in_edges.get(&n))
            .flat_map(|m| m.keys().copied());
        for nb in neighbors.collect::<Vec<_>>() {
            if let std::collections::btree_map::Entry::Vacant(v) = dist.entry(nb) {
                v.insert(d + 1);
                queue.push_back(nb);
            }
        }
    }
    for (&n, &d) in &dist {
        if d > 0 {
            bump(&mut score, n, tier_dependency(d));
        }
    }
    for (&i, &in_title) in &zone.query {
        bump(&mut score, i, tier_query(in_title));
    }
    score.retain(|&i, _| !cat.is_index(i));

    // Orden: relevancia descendente, luego ruta.
    let mut ranked: Vec<(usize, f64)> = score.iter().map(|(&i, &s)| (i, round2(s))).collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| vault[a.0].rel.cmp(&vault[b.0].rel))
    });

    // --- Evidencias y stale (con caché) ----------------------------------
    let mut stale_cache = StaleCache::new(project, &state);
    // Pares (nota, evidencia) secretos omitidos: una nota que sale en `notes`
    // y en `findings` cuenta una sola vez.
    let mut secret_omitted: BTreeSet<(usize, String)> = BTreeSet::new();
    let evidence_of =
        |i: usize, secret_omitted: &mut BTreeSet<(usize, String)>| -> Vec<EvidenceCtx> {
            let mut out = Vec::new();
            for s in &cat.sources[i] {
                if looks_like_secret_path(&s.file, secrets) {
                    secret_omitted.insert((i, s.raw.clone()));
                    continue;
                }
                let status = evidence_status(&project.root, &s.raw);
                let lines = source_lines(&s.raw);
                let file = if s.file.starts_with('/') || s.file.get(1..2) == Some(":") {
                    // Ruta absoluta fuera del proyecto: solo el nombre.
                    s.file.rsplit('/').next().unwrap_or("").to_string()
                } else {
                    s.file.clone()
                };
                out.push(EvidenceCtx {
                    file,
                    line: lines.map(|(a, _)| a),
                    end_line: lines.and_then(|(a, b)| (b != a).then_some(b)),
                    valid: status == EvidenceStatus::Ok,
                });
            }
            out
        };
    let inc = |s: Section| opts.include.contains(&s);

    // --- Notas -----------------------------------------------------------
    let mut notes = Vec::new();
    let total_ranked = ranked.len();
    if inc(Section::Notes) {
        let mut budget = (opts.max_chars > 0).then_some(opts.max_chars);
        let mut chars_cut = false;
        for &(i, rel) in ranked.iter().take(opts.max_notes) {
            let mut content = content_of(&vault[i].text, opts.content);
            let mut truncated = false;
            if let Some(left) = budget.as_mut() {
                let len = content.chars().count();
                if len > *left {
                    content = if *left == 0 {
                        String::new()
                    } else {
                        let kept: String = content.chars().take(*left).collect();
                        format!("{}{TRUNCATION_MARK}", kept.trim_end())
                    };
                    truncated = true;
                    chars_cut = true;
                    *left = 0;
                } else {
                    *left -= len;
                }
            }
            let (stale, stale_reason) = stale_cache.of(&cat, i);
            notes.push(NoteCtx {
                path: vault[i].rel.clone(),
                title: cat.title(i),
                note_type: vault[i].kind().to_string(),
                relevance: rel,
                stale,
                stale_reason,
                content,
                truncated,
                evidence: evidence_of(i, &mut secret_omitted),
            });
        }
        if total_ranked > opts.max_notes {
            warnings.push(t!(
                "context.warn.max_notes",
                shown = opts.max_notes,
                total = total_ranked
            ));
        }
        if chars_cut {
            warnings.push(t!("context.warn.max_chars", max = opts.max_chars));
        }
    }

    // --- Módulos y funcionalidades -------------------------------------
    let rel_of = |i: usize| score.get(&i).copied().unwrap_or(0.0);
    let mut module_set: BTreeSet<usize> = zone.modules.keys().copied().collect();
    for &f in zone.features.iter().chain(&query_features) {
        module_set.extend(cat.feature_module_indexes(f));
    }
    let module_names: BTreeSet<String> = module_set
        .iter()
        .map(|&m| cat.modules[m].name.clone())
        .collect();
    let mut modules = Vec::new();
    if inc(Section::Modules) {
        for &m in &module_set {
            let e = &cat.modules[m];
            let (stale, _) = e
                .note
                .map(|n| stale_cache.of(&cat, n))
                .unwrap_or((false, None));
            modules.push(ModuleCtx {
                id: format!("module:{}", e.name),
                title: e
                    .note
                    .map(|n| cat.title(n))
                    .unwrap_or_else(|| e.name.clone()),
                summary: e
                    .note
                    .map(|n| summary_of(&vault[n].text))
                    .unwrap_or_default(),
                paths: e
                    .paths
                    .iter()
                    .map(|p| {
                        if p.is_empty() {
                            "./".to_string()
                        } else {
                            format!("{p}/")
                        }
                    })
                    .collect(),
                note: e.note.map(|n| vault[n].rel.clone()),
                stale,
            });
        }
        modules.sort_by(|a, b| a.id.cmp(&b.id));
    }

    // Funcionalidades: resueltas, con evidencia en la zona o de un módulo resuelto.
    let mut feature_set: BTreeSet<usize> = zone.features.clone();
    feature_set.extend(&query_features);
    for (i, n) in vault.iter().enumerate() {
        if n.kind() != "feature" {
            continue;
        }
        let in_module = cat
            .feature_modules(i)
            .iter()
            .any(|m| module_names.contains(m));
        if in_module || rel_of(i) >= tier_evidence(1) {
            feature_set.insert(i);
        }
    }
    let mut features = Vec::new();
    if inc(Section::Features) {
        let mut ordered: Vec<usize> = feature_set.iter().copied().collect();
        ordered.sort_by(|&a, &b| {
            rel_of(b)
                .partial_cmp(&rel_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| vault[a].rel.cmp(&vault[b].rel))
        });
        for i in ordered {
            let (stale, _) = stale_cache.of(&cat, i);
            features.push(FeatureCtx {
                id: cat.ref_id(i),
                title: cat.title(i),
                summary: summary_of(&vault[i].text),
                modules: cat
                    .feature_modules(i)
                    .iter()
                    .map(|m| format!("module:{m}"))
                    .collect(),
                note: vault[i].rel.clone(),
                stale,
            });
        }
    }

    // --- Hallazgos ---------------------------------------------------------
    let mut findings = Vec::new();
    if inc(Section::Findings) {
        let mut list: Vec<(usize, f64)> = ranked
            .iter()
            .copied()
            .filter(|&(i, _)| vault[i].kind() == "finding")
            .collect();
        list.sort_by(|a, b| {
            let sev = |i: usize| severity_rank(&cat.field(i, "severity").unwrap_or_default());
            sev(a.0)
                .cmp(&sev(b.0))
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| vault[a.0].rel.cmp(&vault[b.0].rel))
        });
        for (i, _) in list {
            let status = finding_status(&cat.field(i, "state").unwrap_or_default());
            if status != "open" && !opts.all_findings {
                continue;
            }
            let kind = cat.field(i, "kind").unwrap_or_default();
            let (stale, _) = stale_cache.of(&cat, i);
            findings.push(FindingCtx {
                id: cat
                    .fm(i)
                    .and_then(|f| f.id.clone())
                    .unwrap_or_else(|| vault[i].stem().to_string()),
                // Valores del contrato; uno desconocido (que `check` ya
                // reporta) cuenta como `low`.
                severity: cat
                    .field(i, "severity")
                    .filter(|s| severity_rank(s) < 4)
                    .unwrap_or_else(|| "low".into()),
                category: finding_category(&kind).to_string(),
                kind,
                title: cat.title(i),
                summary: summary_of(&vault[i].text),
                status: status.to_string(),
                evidence: evidence_of(i, &mut secret_omitted),
                note: vault[i].rel.clone(),
                stale,
            });
        }
    }

    // --- Tests -------------------------------------------------------------
    let mut tests = TestsCtx::default();
    if inc(Section::Tests) {
        for (i, n) in vault.iter().enumerate() {
            if n.kind() != "test-report" {
                continue;
            }
            let feature = cat
                .field(i, "feature")
                .map(|f| match cat.find_features(&f).first() {
                    Some(&fi) => (Some(fi), cat.ref_id(fi)),
                    None => (None, f),
                })
                .unwrap_or((None, String::new()));
            let related = feature.0.is_some_and(|fi| feature_set.contains(&fi))
                || rel_of(i) >= tier_evidence(1);
            if !related {
                continue;
            }
            let fallback: Vec<String> = cat
                .list(i, "test_files")
                .into_iter()
                .map(|t| normalize_rel_path(&t))
                .filter(|t| !looks_like_secret_path(t, secrets))
                .collect();
            let (covered, missing) = parse_scenarios(body(&n.text));
            for (scenario, refs) in covered {
                let refs: Vec<String> = refs
                    .into_iter()
                    .filter(|r| !looks_like_secret_path(source_file(r), secrets))
                    .collect();
                tests.covered.push(CoveredScenario {
                    feature: feature.1.clone(),
                    scenario,
                    tests: if refs.is_empty() {
                        fallback.clone()
                    } else {
                        refs
                    },
                });
            }
            for scenario in missing {
                tests.missing.push(MissingScenario {
                    feature: feature.1.clone(),
                    scenario,
                });
            }
        }
        // Tests desactivados del inventario dentro de la zona (fase 1: sin línea).
        let mut zone_paths = evidence_paths.clone();
        for &m in &module_set {
            zone_paths.extend(cat.modules[m].paths.iter().cloned());
        }
        if let Some(inv) = &inventory {
            for file in &inv.disabled_tests {
                let file = normalize_rel_path(file);
                let in_zone = zone_paths.iter().any(|p| within(&file, p))
                    || module_set
                        .iter()
                        .any(|&m| cat.modules[m].contains_file(&file));
                if in_zone && !looks_like_secret_path(&file, secrets) {
                    tests.disabled_or_empty.push(DisabledTest {
                        file,
                        line: None,
                        reason: "disabled".into(),
                    });
                }
            }
        }
        tests.disabled_or_empty.sort_by(|a, b| a.file.cmp(&b.file));
        tests.disabled_or_empty.dedup_by(|a, b| a.file == b.file);
    }

    // --- Dependencias (grafo dirigido hasta --depth) ----------------------
    let mut dependencies = DependenciesCtx::default();
    if inc(Section::Dependencies) && opts.depth > 0 {
        dependencies.outgoing = walk(&cat, &seeds, &out_edges, opts.depth, false);
        dependencies.incoming = walk(&cat, &seeds, &in_edges, opts.depth, true);
    }

    // --- Decisiones y glosario --------------------------------------------
    let mut decisions = Vec::new();
    let mut glossary = Vec::new();
    for &(i, _) in &ranked {
        match vault[i].kind() {
            "decision" if inc(Section::Decisions) => {
                let b = body(&vault[i].text);
                let summary = section_paragraph(b, &["decision", "decision tomada"])
                    .map(|s| {
                        clip(
                            &s.split_whitespace().collect::<Vec<_>>().join(" "),
                            SUMMARY_CHARS,
                        )
                    })
                    .unwrap_or_else(|| summary_of(&vault[i].text));
                decisions.push(DecisionCtx {
                    title: cat.title(i),
                    note: vault[i].rel.clone(),
                    summary,
                });
            }
            "glossary" if inc(Section::Glossary) => glossary.push(GlossaryCtx {
                term: cat.title(i),
                definition: summary_of(&vault[i].text),
                note: vault[i].rel.clone(),
            }),
            _ => {}
        }
    }

    // --- Preguntas abiertas -----------------------------------------------
    let relevant_rels: BTreeMap<String, usize> = ranked
        .iter()
        .map(|&(i, _)| (vault[i].rel.clone(), i))
        .collect();
    let mut open_questions = task_questions(project, &state, &module_names, &relevant_rels, &cat);
    open_questions.extend(memory_questions(
        project,
        &module_names,
        &zone.paths,
        &feature_set,
        &cat,
    ));

    // --- Avisos finales ------------------------------------------------------
    if !secret_omitted.is_empty() {
        warnings.push(t!(
            "context.warn.secret_evidence",
            count = secret_omitted.len()
        ));
    }
    for p in &zone.paths {
        let documented = cat
            .sources
            .iter()
            .flatten()
            .any(|s| within(&s.file, p) || within(p, &s.file));
        if !documented {
            warnings.push(t!("context.warn.undocumented", path = display_path(p)));
        }
    }

    let mut ctx = Context {
        schema_version: SCHEMA_VERSION,
        adoc_version: env!("CARGO_PKG_VERSION").to_string(),
        project: ProjectRef {
            id: project.config.project.id.clone(),
            name: project.config.project.name.clone(),
            root: project.root.to_string_lossy().replace('\\', "/"),
            knowledge_commit,
            head_commit,
            generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        },
        resolved_targets: resolved,
        unresolved_targets: unresolved,
        modules,
        features,
        notes,
        findings,
        tests,
        dependencies,
        decisions,
        glossary,
        open_questions,
        warnings,
    };
    let stale = ctx.stale_count();
    if stale > 0 {
        let pending = stale_cache.pending_count();
        ctx.warnings.insert(
            0,
            if pending > 0 {
                t!(
                    "context.warn.stale_pending",
                    count = stale,
                    pending = pending
                )
            } else {
                t!("context.warn.stale", count = stale)
            },
        );
    }
    ctx
}

/// Archivos de código que cambiaron entre `from` y `to` (una llamada a
/// `git diff`), sin el vault, la documentación generada, los archivos que
/// gestiona archeology-doc ni los secretos. `None` si git no conoce `from`.
fn changed_sources_between(project: &Project, from: &str, to: &str) -> Option<Vec<String>> {
    if !from.chars().all(|c| c.is_ascii_hexdigit()) || !to.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let out = crate::gitcmd::git_command(&project.root)
        .args(["diff", "--name-only", "-z", "--relative", from, to, "--"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let knowledge = normalize_rel_path(&project.config.paths.knowledge);
    let docs = normalize_rel_path(&project.config.paths.docs);
    let knowledge = knowledge.trim_end_matches('/');
    let docs = docs.trim_end_matches('/');
    let managed_dirs = [
        knowledge,
        docs,
        ".agents",
        ".archeology-doc",
        ".arqueology-doc",
    ];
    let managed_files = [
        "AGENTS.md",
        "CLAUDE.md",
        "llms.txt",
        ".github/copilot-instructions.md",
        "archeology-doc.toml",
        "arqueology-doc.toml",
    ];
    let secrets = &project.config.scan.secrets_patterns;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .map(normalize_rel_path)
            .filter(|f| !f.is_empty())
            .filter(|f| {
                let ignored = managed_dirs.iter().any(|d| !d.is_empty() && within(f, d))
                    || managed_files.contains(&f.as_str())
                    || (f.starts_with(".cursor/rules/") && f.contains("eology-doc"))
                    || looks_like_secret_path(f, secrets);
                !ignored
            })
            .collect(),
    )
}

fn short(hash: &str) -> String {
    hash.chars().take(8).collect()
}

fn display_path(p: &str) -> String {
    if p.is_empty() {
        ".".into()
    } else {
        p.to_string()
    }
}

/// Resuelve un objetivo y lo añade a la zona.
fn resolve_target(
    cat: &Catalog,
    input: &str,
    cwd: Option<&Path>,
    zone: &mut Zone,
    warnings: &mut Vec<String>,
) -> Option<ResolvedTarget> {
    let trimmed = input.trim();
    let (prefix, name) = if let Some(rest) = trimmed.strip_prefix("module:") {
        (Some("module"), rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("feature:") {
        (Some("feature"), rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("path:") {
        (Some("path"), rest.trim())
    } else {
        (None, trimmed)
    };
    if name.is_empty() {
        return None;
    }
    let ambiguous = |warnings: &mut Vec<String>, matches: &[String]| {
        if matches.len() > 1 {
            warnings.push(t!(
                "context.warn.ambiguous",
                input = input,
                matches = matches.join(", ")
            ));
        }
    };

    if matches!(prefix, None | Some("path")) {
        match cat.resolve_path(name, cwd) {
            PathMatch::Found(p) => {
                let mut matches = Vec::new();
                for (m, e) in cat.modules.iter().enumerate() {
                    if e.touches(&p) {
                        zone.modules.entry(m).or_insert(tier_path_module());
                        matches.push(format!("module:{}", e.name));
                    }
                }
                zone.paths.push(p);
                matches.sort();
                return Some(ResolvedTarget {
                    input: input.to_string(),
                    kind: "path".into(),
                    matches,
                });
            }
            PathMatch::Secret => {
                warnings.push(t!("context.warn.secret_target", input = input));
                return None;
            }
            PathMatch::None if prefix.is_some() => return None,
            PathMatch::None => {}
        }
    }
    if matches!(prefix, None | Some("module")) {
        let found = cat.find_modules(name);
        if !found.is_empty() {
            let matches: Vec<String> = found
                .iter()
                .map(|&m| format!("module:{}", cat.modules[m].name))
                .collect();
            ambiguous(warnings, &matches);
            for m in found {
                zone.modules.insert(m, tier_direct());
            }
            return Some(ResolvedTarget {
                input: input.to_string(),
                kind: "module".into(),
                matches,
            });
        }
        if prefix.is_some() {
            return None;
        }
    }
    let found = cat.find_features(name);
    if found.is_empty() {
        return None;
    }
    let matches: Vec<String> = found.iter().map(|&f| cat.ref_id(f)).collect();
    ambiguous(warnings, &matches);
    zone.features.extend(found);
    Some(ResolvedTarget {
        input: input.to_string(),
        kind: "feature".into(),
        matches,
    })
}

/// Módulos y funcionalidades con nombre parecido a `input`.
fn suggestions(cat: &Catalog, input: &str) -> Vec<String> {
    let name = input
        .trim()
        .trim_start_matches("module:")
        .trim_start_matches("feature:");
    let target = fold(name);
    let mut candidates: Vec<(usize, String)> = Vec::new();
    let mut consider = |label: String, keys: Vec<String>| {
        let best = keys
            .iter()
            .map(|k| {
                let k = fold(k);
                if !target.is_empty() && (k.contains(&target) || target.contains(&k)) {
                    0
                } else {
                    levenshtein(&k, &target)
                }
            })
            .min()
            .unwrap_or(usize::MAX);
        let limit = (target.chars().count() / 2).max(2);
        if best <= limit {
            candidates.push((best, label));
        }
    };
    for m in &cat.modules {
        let mut keys = vec![m.name.clone()];
        keys.extend(m.note.map(|n| cat.title(n)));
        consider(format!("module:{}", m.name), keys);
    }
    for (i, n) in cat.vault.iter().enumerate() {
        if n.kind() == "feature" {
            consider(cat.ref_id(i), vec![n.stem().to_string(), cat.title(i)]);
        }
    }
    candidates.sort();
    candidates.dedup_by(|a, b| a.1 == b.1);
    candidates
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, l)| l)
        .collect()
}

/// Recorrido dirigido del grafo desde la zona: aristas (desde, hacia, nivel).
fn walk(
    cat: &Catalog,
    seeds: &BTreeSet<usize>,
    edges: &BTreeMap<usize, BTreeMap<usize, BTreeSet<String>>>,
    depth: usize,
    reverse: bool,
) -> Vec<DependencyEdge> {
    let mut seen: BTreeSet<usize> = seeds.clone();
    let mut frontier: Vec<usize> = seeds.iter().copied().collect();
    let mut out: BTreeMap<(String, String), DependencyEdge> = BTreeMap::new();
    for level in 1..=depth {
        let mut next = Vec::new();
        for &n in &frontier {
            let Some(targets) = edges.get(&n) else {
                continue;
            };
            for (&t, kinds) in targets {
                if seeds.contains(&t) {
                    continue;
                }
                let (from, to) = if reverse {
                    (cat.ref_id(t), cat.ref_id(n))
                } else {
                    (cat.ref_id(n), cat.ref_id(t))
                };
                out.entry((from.clone(), to.clone()))
                    .or_insert_with(|| DependencyEdge {
                        from,
                        to,
                        depth: level,
                        kinds: kinds.iter().cloned().collect(),
                    });
                if seen.insert(t) {
                    next.push(t);
                }
            }
        }
        frontier = next;
    }
    let mut list: Vec<DependencyEdge> = out.into_values().collect();
    list.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });
    list
}

/// Preguntas de agentes (`state.questions`) de tareas de la zona.
fn task_questions(
    project: &Project,
    state: &State,
    modules: &BTreeSet<String>,
    relevant: &BTreeMap<String, usize>,
    cat: &Catalog,
) -> Vec<QuestionCtx> {
    let mut out = Vec::new();
    for (task, question) in &state.questions {
        let Some(meta) = std::fs::read_to_string(project.tasks_dir().join(format!("{task}.md")))
            .ok()
            .and_then(|t| crate::planner::parse_task(&t))
        else {
            continue;
        };
        let mut related: BTreeSet<String> = BTreeSet::new();
        if let Some(m) = meta.module.as_ref().filter(|m| modules.contains(*m)) {
            related.insert(format!("module:{m}"));
        }
        for p in &meta.produce {
            let p = normalize_rel_path(p);
            for (rel, &i) in relevant {
                if *rel == p || (p.ends_with('/') && rel.starts_with(&p)) {
                    related.insert(cat.ref_id(i));
                }
            }
        }
        if !related.is_empty() {
            out.push(QuestionCtx {
                id: task.clone(),
                question: question.clone(),
                related: related.into_iter().collect(),
            });
        }
    }
    out
}

/// Dudas para el equipo (`memory/open-questions.md`) que mencionan la zona.
fn memory_questions(
    project: &Project,
    modules: &BTreeSet<String>,
    paths: &[String],
    features: &BTreeSet<usize>,
    cat: &Catalog,
) -> Vec<QuestionCtx> {
    let Ok(text) = std::fs::read_to_string(project.memory_dir().join("open-questions.md")) else {
        return Vec::new();
    };
    let entries = body(&text)
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")))
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let mut out = Vec::new();
    for (n, entry) in entries.enumerate() {
        let folded = fold(entry);
        let mut related: BTreeSet<String> = BTreeSet::new();
        for m in modules {
            if mentions_word(&folded, &fold(m)) {
                related.insert(format!("module:{m}"));
            }
        }
        for p in paths.iter().filter(|p| !p.is_empty()) {
            if folded.contains(&fold(p)) {
                related.insert(p.clone());
            }
        }
        for &f in features {
            let fm_id = cat.fm(f).and_then(|x| x.id.clone()).unwrap_or_default();
            let keys = [cat.vault[f].stem().to_string(), fm_id];
            if keys
                .iter()
                .any(|k| !k.is_empty() && mentions_word(&folded, &fold(k)))
            {
                related.insert(cat.ref_id(f));
            }
        }
        if !related.is_empty() {
            out.push(QuestionCtx {
                id: format!("memory#{}", n + 1),
                question: entry.to_string(),
                related: related.into_iter().collect(),
            });
        }
    }
    out
}

/// Estado `stale` de las notas: marcado por `sync` o pendiente (una fuente
/// citada cambió desde el último `sync`). Solo hashea las fuentes de las
/// notas que se piden y nunca lee ni nombra archivos secretos.
struct StaleCache<'a> {
    project: &'a Project,
    state: &'a State,
    hashes: HashMap<String, Option<String>>,
    notes: HashMap<usize, (bool, Option<String>)>,
    pending: BTreeSet<usize>,
}

impl<'a> StaleCache<'a> {
    fn new(project: &'a Project, state: &'a State) -> Self {
        Self {
            project,
            state,
            hashes: HashMap::new(),
            notes: HashMap::new(),
            pending: BTreeSet::new(),
        }
    }

    fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn is_secret(&self, file: &str) -> bool {
        looks_like_secret_path(file, &self.project.config.scan.secrets_patterns)
    }

    /// Lista de fuentes para un motivo: las secretas no se nombran.
    fn describe(&self, files: &[String]) -> String {
        let mut names: Vec<String> = files
            .iter()
            .filter(|f| !self.is_secret(f))
            .cloned()
            .collect();
        if files.iter().any(|f| self.is_secret(f)) {
            names.push(t!("context.stale.secret_source"));
        }
        names.join(", ")
    }

    fn of(&mut self, cat: &Catalog, i: usize) -> (bool, Option<String>) {
        if let Some(v) = self.notes.get(&i) {
            return v.clone();
        }
        let note = &cat.vault[i];
        let result = match note.fm.as_ref() {
            None => (false, None),
            Some(fm) if crate::validate::is_stale(fm) => {
                let reasons = self
                    .state
                    .stale_reasons
                    .get(&note.rel)
                    .filter(|r| !r.is_empty());
                (
                    true,
                    Some(match reasons {
                        Some(r) => t!("context.stale.marked_sources", sources = self.describe(r)),
                        None => t!("context.stale.marked"),
                    }),
                )
            }
            // `sync` no marca las notas bloqueadas.
            Some(fm) if fm.locked || fm.status.as_deref() == Some("locked") => (false, None),
            Some(_) => {
                let mut changed = Vec::new();
                for s in &cat.sources[i] {
                    // Los secretos no se leen (tampoco para hashearlos).
                    if self.is_secret(&s.file) {
                        continue;
                    }
                    let Some(old) = self.state.file_hashes.get(&s.file) else {
                        continue;
                    };
                    let project = self.project;
                    let current = self
                        .hashes
                        .entry(s.file.clone())
                        .or_insert_with(|| project.source_hash(&s.file));
                    if current.as_deref() != Some(old.as_str()) && !changed.contains(&s.file) {
                        changed.push(s.file.clone());
                    }
                }
                if changed.is_empty() {
                    (false, None)
                } else {
                    self.pending.insert(i);
                    (
                        true,
                        Some(t!("context.stale.pending", files = changed.join(", "))),
                    )
                }
            }
        };
        self.notes.insert(i, result.clone());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(fold("Facturación ÁGIL"), "facturacion agil");
        assert!(mentions_word("ver el modulo billing.", "billing"));
        assert!(!mentions_word("billingx", "billing"));
    }

    #[test]
    fn within_and_parent() {
        assert!(within("src/a.rs", "src"));
        assert!(within("src", "src"));
        assert!(!within("srcx/a.rs", "src"));
        assert!(within("x", ""));
        assert_eq!(parent("src/a/b.rs"), "src/a");
        assert_eq!(parent("a.rs"), "");
    }

    #[test]
    fn maps_finding_categories_and_status() {
        assert_eq!(finding_category("stale-doc"), "outdated_doc");
        assert_eq!(finding_category("duplication"), "duplicated_rule");
        assert_eq!(finding_category("config"), "config_conflict");
        assert_eq!(finding_category("dead-code"), "dead_code");
        assert_eq!(finding_category("risk"), "other");
        assert_eq!(finding_status("confirmed"), "open");
        assert_eq!(finding_status(""), "open");
        assert_eq!(finding_status("dismissed"), "dismissed");
    }

    #[test]
    fn relevance_bands_do_not_overlap() {
        // directa > evidencia en la ruta > dependencia 1 > texto > lejanas.
        assert!(tier_path_module() > tier_evidence(3));
        assert!(tier_evidence(1) > tier_dependency(1));
        assert!(tier_dependency(1) > tier_query(true));
        assert!(tier_query(false) > tier_dependency(2));
        assert!(tier_dependency(2) > tier_dependency(3));
    }

    #[test]
    fn parses_scenarios_in_lists_and_tables() {
        let en = "# Scenario coverage\n\n## Covered scenarios\n\n| Scenario | Tests |\n|---|---|\n\
                  | Applies discount | tests/discount_test.py:12, tests/other_test.py |\n\n\
                  ## Missing scenarios\n\n| Scenario | Type | Suggested priority |\n|---|---|---|\n\
                  | Negative amount | edge | high |\n";
        let (c, m) = parse_scenarios(en);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, "Applies discount");
        assert_eq!(
            c[0].1,
            vec!["tests/discount_test.py:12", "tests/other_test.py"]
        );
        assert_eq!(m, vec!["Negative amount"]);
        // Formato antiguo: lista con referencias en la línea.
        let es =
            "## Escenarios cubiertos\n\n- Descuento válido — `tests/d_test.py:3`\n- Sin cupón\n\n\
                  ## Escenarios faltantes\n\n- Cupón vencido\n";
        let (c, m) = parse_scenarios(es);
        assert_eq!(
            c[0],
            (
                "Descuento válido".to_string(),
                vec!["tests/d_test.py:3".to_string()]
            )
        );
        assert_eq!(c[1], ("Sin cupón".to_string(), vec![]));
        assert_eq!(m, vec!["Cupón vencido"]);
    }

    #[test]
    fn excerpt_prefers_summary_region() {
        let text = "---\nid: a\n---\n# A\n\n<!-- archeology-doc:begin summary -->\nResumen corto.\n<!-- archeology-doc:end summary -->\n\nOtro párrafo.\n";
        assert_eq!(content_of(text, ContentMode::Excerpt), "Resumen corto.");
        assert_eq!(summary_of(text), "Resumen corto.");
        assert_eq!(content_of(text, ContentMode::None), "");
        let full = content_of(text, ContentMode::Full);
        assert!(full.contains("Resumen corto.") && full.contains("Otro párrafo."));
        assert!(!full.contains("archeology-doc:begin"));
        let plain =
            "---\nid: b\n---\n# B\n\n> cita\n\nPrimero.\n\n## Sección\n\nSegundo.\n\nTercero.\n";
        assert_eq!(
            content_of(plain, ContentMode::Excerpt),
            "Primero.\n\nSegundo."
        );
    }

    /// Proyecto temporal con un vault de prueba.
    struct Fixture {
        root: PathBuf,
        project: Project,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).ok();
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("adoc-zone-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("archeology-doc.toml"),
            toml::to_string(&crate::config::ArcheologyConfig::new("z", "es")).unwrap(),
        )
        .unwrap();
        std::env::set_var("ARCHEOLOGY_DOC_CONFIG_DIR", root.join(".cfg"));
        write(&root, "src/billing/invoice.py", "a\nb\nc\nd\ne\n");
        write(&root, "src/billing/tax.py", "x\n");
        write(&root, "src/auth/login.py", "x\n");
        write(&root, "src/billing/.env", "SECRET=1\n");
        let k = "knowledge";
        write(
            &root,
            &format!("{k}/modules/billing.md"),
            "---\nid: mod-billing\ntype: module\ntitle: Facturación\nsources: [src/billing/invoice.py:1]\n---\n# Facturación\n\n<!-- archeology-doc:begin summary -->\nCobros y facturas.\n<!-- archeology-doc:end summary -->\n",
        );
        write(
            &root,
            &format!("{k}/modules/auth.md"),
            "---\nid: mod-auth\ntype: module\ntitle: Auth\nsources: [src/auth/login.py:1]\n---\n# Auth\n\nLogin.\n",
        );
        write(
            &root,
            &format!("{k}/features/discount.md"),
            "---\nid: feat-discount\ntype: feature\ntitle: Descuento por volumen\nmodules: [billing]\nsources: [src/billing/invoice.py:2-3, src/billing/.env]\n---\n# Descuento\n\nAplica descuentos. Ver [[auth]].\n",
        );
        write(
            &root,
            &format!("{k}/findings/limit.md"),
            "---\nid: find-limit\ntype: finding\ntitle: Tope distinto\nkind: contradiction\nseverity: high\nstate: open\nrelated: [feat-discount]\nsources: [src/billing/invoice.py:4-99]\n---\n# Tope\n\n## Description\n\nEl tope difiere.\n",
        );
        write(
            &root,
            &format!("{k}/findings/old.md"),
            "---\nid: find-old\ntype: finding\ntitle: Viejo\nkind: dead-code\nseverity: critical\nstate: resolved\nsources: [src/billing/tax.py:1]\n---\n# Viejo\n",
        );
        write(
            &root,
            &format!("{k}/glossary/iva.md"),
            "---\nid: glossary-iva\ntype: glossary\ntitle: IVA\nsources: [src/billing/tax.py:1]\n---\n# IVA\n\nImpuesto al valor agregado.\n",
        );
        let project = Project::open(&root).unwrap();
        Fixture { root, project }
    }

    fn opts(targets: &[&str]) -> Options {
        Options {
            targets: targets.iter().map(|s| s.to_string()).collect(),
            ..Options::default()
        }
    }

    fn paths(ctx: &Context) -> Vec<&str> {
        ctx.notes.iter().map(|n| n.path.as_str()).collect()
    }

    #[test]
    fn resolves_each_target_kind_in_order() {
        let fx = fixture("resolve");
        let p = &fx.project;
        // Archivo: notas con evidencia en él.
        let c = build(p, &opts(&["src/billing/invoice.py"]));
        assert_eq!(c.resolved_targets[0].kind, "path");
        assert!(paths(&c).contains(&"knowledge/features/discount.md"));
        // Separadores de Windows normalizados.
        let c = build(p, &opts(&["src\\billing\\tax.py"]));
        assert_eq!(c.resolved_targets[0].kind, "path");
        assert!(paths(&c).contains(&"knowledge/glossary/iva.md"));
        // Módulo por id de nota, por prefijo y por título normalizado.
        let c = build(p, &opts(&["module:billing"]));
        assert_eq!(c.resolved_targets[0].matches, vec!["module:billing"]);
        let c = build(p, &opts(&["facturacion"]));
        assert_eq!(c.resolved_targets[0].kind, "module");
        assert_eq!(c.notes[0].path, "knowledge/modules/billing.md");
        assert_eq!(c.notes[0].relevance, 1.0);
        // Funcionalidad por id y por título.
        let c = build(p, &opts(&["feature:feat-discount"]));
        assert_eq!(c.resolved_targets[0].matches, vec!["feature:discount"]);
        let c = build(p, &opts(&["Descuento por Volumen"]));
        assert_eq!(c.resolved_targets[0].kind, "feature");
        // Inexistente: sin resolver, con sugerencia.
        let c = build(p, &opts(&["module:bilin"]));
        assert!(!c.resolved_any());
        assert_eq!(c.unresolved_targets, vec!["module:bilin"]);
        assert!(c.warnings.iter().any(|w| w.contains("module:billing")));
        // Fuera del proyecto o con `..`: sin resolver.
        let c = build(p, &opts(&["../x"]));
        assert!(!c.resolved_any());
    }

    #[test]
    fn ambiguous_titles_return_all_matches_with_warning() {
        let fx = fixture("ambiguous");
        write(
            &fx.root,
            "knowledge/features/discount2.md",
            "---\nid: feat-d2\ntype: feature\ntitle: Descuento por volumen\nsources: [src/billing/tax.py:1]\n---\n",
        );
        let c = build(&fx.project, &opts(&["feature:descuento por volumen"]));
        assert_eq!(c.resolved_targets[0].matches.len(), 2);
        assert!(c.warnings.iter().any(|w| w.contains("feature:discount2")));
    }

    #[test]
    fn evidence_validity_and_secrets() {
        let fx = fixture("evidence");
        let c = build(&fx.project, &opts(&["module:billing"]));
        let finding = c.findings.iter().find(|f| f.id == "find-limit").unwrap();
        // 4-99 en un archivo de 5 líneas: inválida, como en `adoc check`.
        assert!(!finding.evidence[0].valid);
        assert_eq!(finding.category, "contradiction");
        // El hallazgo resuelto no aparece sin --all-findings.
        assert!(c.findings.iter().all(|f| f.id != "find-old"));
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains(".env"), "{json}");
        assert!(c.warnings.iter().any(|w| w.contains('1')));
        let feat = c.notes.iter().find(|n| n.note_type == "feature").unwrap();
        assert_eq!(feat.evidence.len(), 1);
        assert_eq!(
            (
                feat.evidence[0].line,
                feat.evidence[0].end_line,
                feat.evidence[0].valid
            ),
            (Some(2), Some(3), true)
        );
        // Un objetivo secreto no se resuelve.
        let c = build(&fx.project, &opts(&["src/billing/.env"]));
        assert!(!c.resolved_any());
        let mut all = opts(&["module:billing"]);
        all.all_findings = true;
        let c = build(&fx.project, &all);
        assert_eq!(c.findings[0].id, "find-old", "críticos primero");
    }

    #[test]
    fn depth_controls_linked_notes_and_dependencies() {
        let fx = fixture("depth");
        let mut o = opts(&["feature:discount"]);
        o.depth = 0;
        let c = build(&fx.project, &o);
        assert!(c.dependencies.outgoing.is_empty());
        assert!(!paths(&c).contains(&"knowledge/modules/auth.md"));
        o.depth = 1;
        let c = build(&fx.project, &o);
        assert!(c
            .dependencies
            .outgoing
            .iter()
            .any(|e| e.to == "module:auth" && e.depth == 1));
        let auth = c
            .notes
            .iter()
            .find(|n| n.path == "knowledge/modules/auth.md")
            .unwrap();
        assert_eq!(auth.relevance, 0.6);
        let c = build(&fx.project, &opts(&["module:auth"]));
        assert!(c
            .dependencies
            .incoming
            .iter()
            .any(|e| e.from == "feature:discount" && e.to == "module:auth"));
    }

    #[test]
    fn ordering_is_deterministic_and_limits_apply() {
        let fx = fixture("order");
        let mut o = opts(&["src/billing/"]);
        o.max_notes = 2;
        o.max_chars = 10;
        let a = build(&fx.project, &o);
        let b = build(&fx.project, &o);
        assert_eq!(paths(&a), paths(&b));
        assert_eq!(a.notes.len(), 2);
        assert!(a.notes.windows(2).all(|w| w[0].relevance >= w[1].relevance));
        assert!(a.notes.iter().any(|n| n.truncated));
        assert!(
            a.notes[0].content.ends_with(TRUNCATION_MARK)
                || a.notes[0].content.chars().count() <= 10
        );
        assert!(a.warnings.len() >= 2);
    }

    #[test]
    fn stale_marked_and_pending() {
        let fx = fixture("stale");
        let mut state = State::default();
        state
            .file_hashes
            .insert("src/auth/login.py".into(), "hash-anterior".into());
        state.save(&fx.project.state_file()).unwrap();
        let c = build(&fx.project, &opts(&["module:auth"]));
        let auth = &c.notes[0];
        assert!(auth.stale);
        assert!(auth
            .stale_reason
            .as_deref()
            .unwrap()
            .contains("src/auth/login.py"));
        assert_eq!(c.stale_count(), 1);
        crate::index::mark_stale(&fx.root.join("knowledge/modules/billing.md")).unwrap();
        let c = build(&fx.project, &opts(&["module:billing"]));
        assert!(c.modules[0].stale);
    }

    /// Rendimiento con un vault grande (~2000 notas). Manual:
    /// `cargo test -p archeology-core zone_perf -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn zone_perf_with_2000_notes() {
        let fx = fixture("perf");
        for f in 0..200 {
            write(
                &fx.root,
                &format!("src/m{}/f{f}.py", f % 40),
                &"x = 1\n".repeat(50),
            );
        }
        for m in 0..40 {
            write(
                &fx.root,
                &format!("knowledge/modules/m{m}.md"),
                &format!("---\nid: mod-m{m}\ntype: module\ntitle: Módulo {m}\nsources: [src/m{m}/f{m}.py:1-10]\n---\n# M{m}\n\nTexto del módulo.\n"),
            );
        }
        for i in 0..1500 {
            let f = i % 200;
            write(
                &fx.root,
                &format!("knowledge/features/f{i}.md"),
                &format!("---\nid: feat-f{i}\ntype: feature\ntitle: Funcionalidad {i}\nmodules: [m{}]\nsources: [src/m{}/f{f}.py:{}-{}]\n---\n# F{i}\n\nVer [[m{}]] y [[f{}]].\n\n{}\n", f % 40, f % 40, 1 + i % 40, 5 + i % 40, (i + 1) % 40, (i + 7) % 1500, "Lorem ipsum dolor sit amet. ".repeat(20)),
            );
        }
        for i in 0..460 {
            write(
                &fx.root,
                &format!("knowledge/findings/x{i}.md"),
                &format!("---\nid: find-x{i}\ntype: finding\ntitle: Hallazgo {i}\nkind: risk\nseverity: medium\nstate: open\nrelated: [feat-f{i}]\nsources: [src/m{}/f{}.py:3]\n---\n# X\n\nRiesgo.\n", i % 40, i % 200),
            );
        }
        let vault_size = crate::index::vault_notes(&fx.project).len();
        assert!(vault_size >= 2000, "{vault_size}");
        for (label, target) in [
            ("module", "module:m3"),
            ("path", "src/m7/"),
            ("feature", "feature:f42"),
        ] {
            let mut o = opts(&[target]);
            o.depth = 2;
            let start = std::time::Instant::now();
            let c = build(&fx.project, &o);
            let ms = start.elapsed().as_millis();
            eprintln!("zone perf: {vault_size} notas · {label} {target}: {ms} ms · {} notas · {} hallazgos", c.notes.len(), c.findings.len());
            assert!(c.resolved_any());
        }
    }

    // --- Regresiones de la revisión ---------------------------------------

    #[test]
    fn negative_scenario_headings_are_missing() {
        for heading in [
            "## Uncovered scenarios",
            "## Not covered",
            "## Escenarios no cubiertos",
            "## No cubiertos",
            "## Sin cubrir",
            "## Missing scenarios",
        ] {
            let body = format!("{heading}\n\n- Escenario X\n");
            let (c, m) = parse_scenarios(&body);
            assert!(c.is_empty(), "{heading}: {c:?}");
            assert_eq!(m, vec!["Escenario X"], "{heading}");
        }
        let (c, _) = parse_scenarios("## Covered scenarios\n\n- Ok\n");
        assert_eq!(c.len(), 1);
        let (c, _) = parse_scenarios("## Escenarios cubiertos\n\n- Ok\n");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn stale_never_reads_or_names_secret_sources() {
        let fx = fixture("stale-secret");
        write(
            &fx.root,
            "knowledge/modules/vault.md",
            "---\nid: mod-vault\ntype: module\ntitle: Vault\nsources: [src/billing/.env, src/billing/tax.py:1]\n---\n# Vault\n",
        );
        let mut state = State::default();
        // Hash viejo del secreto: si se leyera, la nota quedaría pendiente.
        state
            .file_hashes
            .insert("src/billing/.env".into(), "viejo".into());
        state.save(&fx.project.state_file()).unwrap();
        let vault = load_vault(&fx.project);
        let inv = None;
        let cat = Catalog::new(&fx.project, &vault, inv);
        let i = vault.iter().position(|n| n.id == "modules/vault").unwrap();
        let mut cache = StaleCache::new(&fx.project, &state);
        assert_eq!(cache.of(&cat, i), (false, None));
        assert!(!cache.hashes.contains_key("src/billing/.env"));
        // Marcada por sync con un secreto entre los motivos: no se nombra.
        crate::index::mark_stale(&fx.root.join("knowledge/modules/vault.md")).unwrap();
        state.stale_reasons.insert(
            "knowledge/modules/vault.md".into(),
            vec!["src/billing/.env".into(), "src/billing/tax.py".into()],
        );
        state.save(&fx.project.state_file()).unwrap();
        let c = build(&fx.project, &opts(&["module:vault"]));
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains(".env"), "{json}");
        let reason = c.notes[0].stale_reason.clone().unwrap();
        assert!(reason.contains("src/billing/tax.py"), "{reason}");
        assert!(
            reason.contains(&t!("context.stale.secret_source")),
            "{reason}"
        );
    }

    #[test]
    fn cwd_relative_parent_paths_and_project_root() {
        let fx = fixture("cwd");
        let mut o = opts(&["../billing"]);
        o.cwd = Some(fx.root.join("src/auth"));
        let c = build(&fx.project, &o);
        assert_eq!(c.resolved_targets[0].kind, "path", "{:?}", c.warnings);
        assert!(paths(&c).contains(&"knowledge/modules/billing.md"));
        // Fuera de la raíz sigue sin resolverse.
        let mut o = opts(&["../../.."]);
        o.cwd = Some(fx.root.join("src/auth"));
        assert!(!build(&fx.project, &o).resolved_any());
        // La raíz absoluta (con o sin `/` final) es todo el proyecto.
        let root = fx.root.to_string_lossy().to_string();
        for target in [root.clone(), format!("{root}/")] {
            let c = build(&fx.project, &opts(&[&target]));
            assert_eq!(c.resolved_targets[0].kind, "path", "{target}");
            assert!(c.notes.len() >= 4, "{target}");
        }
    }

    #[test]
    fn feature_modules_resolve_aliases() {
        let fx = fixture("alias");
        write(
            &fx.root,
            "knowledge/features/refund.md",
            "---\nid: feat-refund\ntype: feature\ntitle: Reembolso\nmodules: [mod-billing]\nsources: [src/auth/login.py:1]\n---\n# Reembolso\n",
        );
        let c = build(&fx.project, &opts(&["module:billing"]));
        let refund = c
            .features
            .iter()
            .find(|f| f.id == "feature:refund")
            .expect("la funcionalidad con alias pertenece al módulo");
        assert_eq!(refund.modules, vec!["module:billing"]);
        let c = build(&fx.project, &opts(&["feature:refund"]));
        assert_eq!(c.modules[0].id, "module:billing");
    }

    #[test]
    fn query_only_seeds_the_zone() {
        let fx = fixture("query");
        let mut o = Options {
            query: Some("descuentos".into()),
            ..Options::default()
        };
        let c = build(&fx.project, &o);
        assert!(c.resolved_any());
        assert!(
            c.features.iter().any(|f| f.id == "feature:discount"),
            "{:?}",
            c.features
        );
        assert!(c.modules.iter().any(|m| m.id == "module:billing"));
        assert!(!c.dependencies.outgoing.is_empty());
        o.depth = 0;
        let c = build(&fx.project, &o);
        assert!(c.dependencies.outgoing.is_empty());
        assert!(!paths(&c).contains(&"knowledge/modules/auth.md"));
    }

    #[test]
    fn secret_evidence_is_counted_once_per_note() {
        let fx = fixture("secret-count");
        write(
            &fx.root,
            "knowledge/findings/leak.md",
            "---\nid: find-leak\ntype: finding\ntitle: Fuga\nkind: risk\nseverity: high\nstate: open\nsources: [src/billing/credentials.json, src/billing/tax.py:1]\n---\n# Fuga\n",
        );
        write(&fx.root, "src/billing/credentials.json", "{}\n");
        let mut o = opts(&["src/billing/tax.py"]);
        o.include = [Section::Notes, Section::Findings].into_iter().collect();
        // Sin dependencias: solo la nota del hallazgo cita un secreto.
        o.depth = 0;
        let c = build(&fx.project, &o);
        // La nota sale en `notes` y en `findings`: una sola evidencia omitida.
        assert!(c.findings.iter().any(|f| f.id == "find-leak"));
        assert!(c
            .notes
            .iter()
            .any(|n| n.path == "knowledge/findings/leak.md"));
        let expected = t!("context.warn.secret_evidence", count = 1);
        assert!(c.warnings.contains(&expected), "{:?}", c.warnings);
    }
}
