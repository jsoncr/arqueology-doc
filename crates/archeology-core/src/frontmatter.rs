//! Parsing y validación del frontmatter YAML de las notas (§11).

use crate::t;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Frontmatter parseado de una nota: campos tipados comunes + resto en mapa.
#[derive(Debug, Clone)]
pub struct NoteFrontmatter {
    pub id: Option<String>,
    pub note_type: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub confidence: Option<String>,
    pub sources: Vec<String>,
    pub locked: bool,
    pub fields: serde_yaml::Mapping,
}

/// Tipos de nota válidos (§11.1).
pub const NOTE_TYPES: &[&str] = &[
    "overview",
    "module",
    "feature",
    "finding",
    "test-report",
    "spec-report",
    "glossary",
    "decision",
    "memory",
    "moc",
    "faq",
];

const STATUSES: &[&str] = &["draft", "reviewed", "stale", "locked"];
const CONFIDENCES: &[&str] = &["high", "medium", "low"];
const FINDING_KINDS: &[&str] = &[
    "contradiction",
    "duplication",
    "dead-code",
    "risk",
    "tech-debt",
    "stale-doc",
    "config",
    "question",
    "spec-drift",
    "unspecified-behavior",
    "stale-change",
];
const SEVERITIES: &[&str] = &["critical", "high", "medium", "low"];
const FINDING_STATES: &[&str] = &["open", "confirmed", "dismissed", "resolved"];

/// Extrae el bloque `--- ... ---` del inicio de un documento Markdown.
pub fn extract_frontmatter(text: &str) -> Option<(serde_yaml::Mapping, &str)> {
    let (yaml, body_start) = frontmatter_range(text)?;
    let mapping: serde_yaml::Mapping = serde_yaml::from_str(&text[yaml]).ok()?;
    Some((mapping, &text[body_start..]))
}

/// Rango en bytes del YAML dentro de `text` (entre la línea `---` de apertura
/// y la de cierre) y posición donde empieza el cuerpo. Admite BOM y líneas en
/// blanco antes de la apertura, CRLF y `...` como cierre.
pub fn frontmatter_range(text: &str) -> Option<(std::ops::Range<usize>, usize)> {
    let trimmed = text.trim_start_matches(['\u{feff}', '\n', '\r', ' ', '\t']);
    let lead = text.len() - trimmed.len();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    // La línea de apertura debe ser solo "---".
    let first_nl = after_open.find('\n')?;
    if !after_open[..first_nl].trim().is_empty() {
        return None;
    }
    let yaml_start = lead + 3 + first_nl + 1;
    let (yaml_end, body_start) = find_closing(&text[yaml_start..])?;
    Some((yaml_start..yaml_start + yaml_end, yaml_start + body_start))
}

/// Fija `key: value` en el frontmatter sin tocar nada fuera del YAML. Si la
/// clave no existe, se añade al final del frontmatter. Conserva el BOM y el
/// tipo de salto de línea. Sin frontmatter, devuelve el texto tal cual.
pub fn set_field(text: &str, key: &str, value: &str) -> String {
    let Some((yaml, _)) = frontmatter_range(text) else {
        return text.to_string();
    };
    let eol = if text[yaml.clone()].contains("\r\n") || text[yaml.end..].starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let prefix = format!("{key}:");
    let mut offset = yaml.start;
    for line in text[yaml.clone()].split_inclusive('\n') {
        // Solo claves de primer nivel (sin sangría).
        if line.starts_with(&prefix) {
            let content_end = offset + line.trim_end_matches(['\n', '\r']).len();
            return format!("{}{key}: {value}{}", &text[..offset], &text[content_end..]);
        }
        offset += line.len();
    }
    format!(
        "{}{key}: {value}{eol}{}",
        &text[..yaml.end],
        &text[yaml.end..]
    )
}

/// Devuelve (fin del YAML, inicio del body): posición donde empieza la
/// línea de cierre y donde termina.
fn find_closing(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let stripped = line.trim_end();
        if stripped == "---" || stripped == "..." {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

impl NoteFrontmatter {
    pub fn parse(text: &str) -> Option<Self> {
        let (map, _) = extract_frontmatter(text)?;
        let get_str = |key: &str| -> Option<String> {
            map.get(serde_yaml::Value::String(key.into()))
                .and_then(|v| v.as_str().map(String::from))
        };
        let sources = map
            .get(serde_yaml::Value::String("sources".into()))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Ante la duda se bloquea: `locked: yes` o `"true"` también protegen la nota.
        let locked = match map.get(serde_yaml::Value::String("locked".into())) {
            Some(serde_yaml::Value::Bool(b)) => *b,
            Some(serde_yaml::Value::String(s)) => matches!(
                s.trim().to_lowercase().as_str(),
                "true" | "yes" | "y" | "on" | "si" | "sí"
            ),
            _ => false,
        };
        Some(Self {
            id: get_str("id"),
            note_type: get_str("type"),
            title: get_str("title"),
            status: get_str("status"),
            confidence: get_str("confidence"),
            sources,
            locked,
            fields: map,
        })
    }

    pub fn get(&self, key: &str) -> Option<&serde_yaml::Value> {
        self.fields.get(serde_yaml::Value::String(key.into()))
    }
}

/// Error de validación de una nota.
#[derive(Debug, Clone)]
pub struct NoteIssue {
    pub path: String,
    pub severity: IssueSeverity,
    pub message: String,
    /// Qué validación lo produjo (agrupa la salida de `check`).
    pub kind: IssueKind,
    /// Línea del problema dentro del archivo, si se conoce (1-based).
    pub line: Option<usize>,
}

/// Validación que produjo un [`NoteIssue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// Frontmatter: campos obligatorios, tipos, valores no estándar.
    Frontmatter,
    /// Evidencias `sources:` (archivo existente, dentro del proyecto, rango).
    Evidence,
    /// Wikilinks `[[...]]` sin nota destino.
    Wikilink,
    /// IDs repetidos y hallazgos equivalentes.
    Duplicate,
    /// Notas marcadas `status: stale` por `sync`.
    Stale,
    /// Nota ilegible (no UTF-8, permisos).
    Unreadable,
    /// Artefactos que una tarea declara en `# Produce`.
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
}

/// Valida el frontmatter de una nota según su tipo (RF-130).
pub fn validate_note(path: &Path, text: &str, project_root: &Path) -> Vec<NoteIssue> {
    let display = path.display().to_string();
    let mut issues = Vec::new();
    let err = |display: &String, msg: String| NoteIssue {
        path: display.clone(),
        severity: IssueSeverity::Error,
        message: msg,
        kind: IssueKind::Frontmatter,
        line: None,
    };
    let warn = |display: &String, msg: String| NoteIssue {
        path: display.clone(),
        severity: IssueSeverity::Warning,
        message: msg,
        kind: IssueKind::Frontmatter,
        line: None,
    };
    // Las evidencias se agrupan aparte en la salida de `check`.
    let evidence = |display: &String, msg: String| NoteIssue {
        kind: IssueKind::Evidence,
        ..err(display, msg)
    };

    let fm = match NoteFrontmatter::parse(text) {
        Some(f) => f,
        None => {
            issues.push(err(&display, t!("core.note.no_frontmatter")));
            return issues;
        }
    };

    if fm.id.as_deref().unwrap_or("").is_empty() {
        issues.push(err(&display, t!("core.note.missing_id")));
    }
    match fm.note_type.as_deref() {
        None => issues.push(err(&display, t!("core.note.missing_type"))),
        Some(t) if !NOTE_TYPES.contains(&t) => issues.push(err(
            &display,
            t!(
                "core.note.unknown_type",
                value = t,
                valid = NOTE_TYPES.join(", ")
            ),
        )),
        _ => {}
    }
    if let Some(s) = fm.status.as_deref() {
        if !STATUSES.contains(&s) {
            issues.push(warn(
                &display,
                t!("core.note.nonstandard_status", value = s),
            ));
        }
    }
    if let Some(c) = fm.confidence.as_deref() {
        if !CONFIDENCES.contains(&c) {
            issues.push(warn(
                &display,
                t!("core.note.nonstandard_confidence", value = c),
            ));
        }
    }

    // Memoria estable vs temporal (RF-103).
    if fm.note_type.as_deref() == Some("memory") {
        if let Some(m) = fm.get("memory").and_then(|v| v.as_str()) {
            if m != "stable" && m != "temporary" {
                issues.push(warn(
                    &display,
                    t!("core.note.nonstandard_memory", value = m),
                ));
            }
        }
    }

    // Validación por tipo.
    if fm.note_type.as_deref() == Some("finding") {
        match fm.get("kind").and_then(|v| v.as_str()) {
            Some(k) if FINDING_KINDS.contains(&k) => {}
            Some(k) => issues.push(err(&display, t!("core.note.invalid_kind", value = k))),
            None => issues.push(err(&display, t!("core.note.finding_without_kind"))),
        }
        match fm.get("severity").and_then(|v| v.as_str()) {
            Some(s) if SEVERITIES.contains(&s) => {}
            Some(s) => issues.push(err(&display, t!("core.note.invalid_severity", value = s))),
            None => issues.push(err(&display, t!("core.note.finding_without_severity"))),
        }
        match fm.get("state").and_then(|v| v.as_str()) {
            Some(s) if FINDING_STATES.contains(&s) => {}
            Some(s) => issues.push(err(&display, t!("core.note.invalid_state", value = s))),
            None => issues.push(err(&display, t!("core.note.finding_without_state"))),
        }
        if fm.sources.is_empty() {
            issues.push(evidence(&display, t!("core.note.finding_without_sources")));
        }
    }

    // Verificar que las evidencias existen: archivo y rango de líneas (RF-082).
    for src in &fm.sources {
        let message = match evidence_status(project_root, src) {
            EvidenceStatus::Ok => continue,
            EvidenceStatus::Outside => t!("core.note.source_outside", source = src),
            EvidenceStatus::NotFound => t!("core.note.source_not_found", source = src),
            EvidenceStatus::OutOfRange { total } => {
                t!("core.note.source_out_of_range", source = src, total = total)
            }
        };
        issues.push(evidence(&display, message));
    }

    // Features y findings requieren al menos una evidencia (§17).
    if fm.note_type.as_deref() == Some("feature") && fm.sources.is_empty() {
        issues.push(evidence(&display, t!("core.note.feature_without_sources")));
    }

    issues
}

/// Destino de una evidencia `sources:` respecto de la raíz del proyecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceTarget {
    /// Ruta absoluta, con `..` o que sale del proyecto a través de un symlink.
    Outside,
    /// Dentro del proyecto, pero no existe (o no es un archivo).
    NotFound,
    /// Archivo existente dentro del proyecto (ruta completa).
    File(std::path::PathBuf),
}

/// Validez de una evidencia `sources` según `adoc check` (RF-082): la usan
/// `check` (vía [`validate_note`]) y `adoc context` (`evidence[].valid`).
/// Solo lee el archivo citado para contar líneas si la evidencia tiene rango.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStatus {
    /// Archivo existente y rango (si lo hay) dentro del archivo.
    Ok,
    /// Ruta absoluta, con `..` o que sale del proyecto.
    Outside,
    /// El archivo no existe.
    NotFound,
    /// Rango inválido o más allá del final (`total` líneas).
    OutOfRange { total: usize },
}

/// Calcula el [`EvidenceStatus`] de `src` (`ruta`, `ruta:10`, `ruta:10-20`).
pub fn evidence_status(project_root: &Path, src: &str) -> EvidenceStatus {
    let full = match resolve_source(project_root, src) {
        SourceTarget::Outside => return EvidenceStatus::Outside,
        SourceTarget::NotFound => return EvidenceStatus::NotFound,
        SourceTarget::File(full) => full,
    };
    if let Some((a, b)) = crate::index::source_lines(src) {
        let total = file_line_count(&full);
        if a == 0 || a > b || b > total {
            return EvidenceStatus::OutOfRange { total };
        }
    }
    EvidenceStatus::Ok
}

/// Resuelve el archivo de una evidencia (`ruta`, `ruta:10`, `ruta:10-20`).
///
/// Rutas escritas por agentes: absolutas, con `..` o a través de un symlink
/// podrían apuntar fuera del proyecto (y leerse para contar líneas). Se
/// rechazan sin tocarlas.
pub fn resolve_source(project_root: &Path, src: &str) -> SourceTarget {
    let file = crate::index::source_file(src);
    let full = project_root.join(file);
    let lexically_inside = !Path::new(file).is_absolute()
        && Path::new(file).components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        });
    if !lexically_inside || !crate::project::is_inside(project_root, &full) {
        return SourceTarget::Outside;
    }
    if !full.is_file() {
        return SourceTarget::NotFound;
    }
    SourceTarget::File(full)
}

/// Líneas de un contenido, como las cuenta el validador de evidencias:
/// cada `\n` cierra una línea y un último fragmento sin `\n` también cuenta.
/// `""` → 0, `"a\nb\n"` → 2, `"a\nb"` → 2, `"a\r\nb\r\n"` → 2.
pub fn count_lines(bytes: &[u8]) -> usize {
    let n = bytes.iter().filter(|&&c| c == b'\n').count();
    if bytes.last().is_some_and(|&c| c != b'\n') {
        n + 1
    } else {
        n
    }
}

/// Líneas de un archivo según [`count_lines`] (0 si no se puede leer).
pub fn file_line_count(path: &Path) -> usize {
    std::fs::read(path).map(|b| count_lines(&b)).unwrap_or(0)
}

/// Evidencia con el final del rango recortado a `total` líneas, si el rango
/// empieza dentro del archivo y solo se pasa por el final:
/// `a.ejs:1-400` con 339 líneas → `a.ejs:1-339`; `a.ejs:339-400` → `a.ejs:339`.
/// `None` si el rango ya es válido o no se puede reparar (inicio 0, inicio
/// mayor que el final, inicio más allá del final del archivo, sin rango).
pub fn clamp_source(src: &str, total: usize) -> Option<String> {
    let (a, b) = crate::index::source_lines(src)?;
    if a == 0 || a > b || a > total || b <= total {
        return None;
    }
    let file = crate::index::source_file(src);
    Some(if a == total {
        format!("{file}:{a}")
    } else {
        format!("{file}:{a}-{total}")
    })
}

/// Sustituye evidencias de `sources:` en el frontmatter con una edición
/// mínima: solo cambia el texto de cada evidencia dentro del bloque
/// `sources:` (lista en bloque o en línea); conserva orden, comillas,
/// comentarios, saltos de línea y el resto del documento.
///
/// Devuelve `None` si no hay frontmatter, no hay bloque `sources:` o el
/// resultado no se parsea con exactamente las evidencias esperadas (p. ej.
/// cadenas YAML con escapes): ante la duda no se toca la nota.
pub fn replace_sources(text: &str, changes: &[(String, String)]) -> Option<String> {
    let (yaml, _) = frontmatter_range(text)?;
    let before = NoteFrontmatter::parse(text)?;
    // Bloque `sources:`: su línea y las siguientes con sangría, `-`,
    // comentarios o vacías (hasta la siguiente clave de primer nivel).
    let mut start = None;
    let mut end = yaml.end;
    let mut offset = yaml.start;
    for line in text[yaml.clone()].split_inclusive('\n') {
        if start.is_none() {
            if line.starts_with("sources:") {
                start = Some(offset);
            }
        } else if !line.starts_with([' ', '\t', '-', '#', '\r', '\n']) {
            end = offset;
            break;
        }
        offset += line.len();
    }
    let start = start?;
    let mut region = text[start..end].to_string();
    for (from, to) in changes {
        region = replace_bounded(&region, from, to);
    }
    let out = format!("{}{region}{}", &text[..start], &text[end..]);
    // Verificación: mismas evidencias con las sustituciones y el resto igual.
    let expected: Vec<String> = before
        .sources
        .iter()
        .map(|s| {
            changes
                .iter()
                .find(|(f, _)| f == s)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| s.clone())
        })
        .collect();
    let after = NoteFrontmatter::parse(&out)?;
    let mut rest_before = before.fields.clone();
    let mut rest_after = after.fields.clone();
    rest_before.remove(serde_yaml::Value::String("sources".into()));
    rest_after.remove(serde_yaml::Value::String("sources".into()));
    (after.sources == expected && rest_before == rest_after).then_some(out)
}

/// Reemplaza en `text` cada aparición de `from` delimitada como un valor
/// YAML completo (precedida por espacio, comilla, `[` o `,`; seguida por
/// espacio, comilla, `,`, `]` o el final).
fn replace_bounded(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(from) {
        let prev = out
            .chars()
            .last()
            .filter(|_| pos == 0)
            .or_else(|| rest[..pos].chars().last());
        let next = rest[pos + from.len()..].chars().next();
        let prev_ok =
            prev.is_some_and(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '[' | ','));
        let next_ok = next.is_none_or(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ']'));
        out.push_str(&rest[..pos]);
        if prev_ok && next_ok {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        rest = &rest[pos + from.len()..];
    }
    out.push_str(rest);
    out
}

/// Fila de `review`: hallazgo abierto con su severidad.
#[derive(Debug, Deserialize)]
pub struct FindingRow {
    pub id: Option<String>,
    pub title: Option<String>,
    pub severity: Option<String>,
    pub state: Option<String>,
    pub kind: Option<String>,
}

/// Convierte `serde_yaml::Mapping` a un mapa plano de strings (para conteos).
pub fn mapping_to_map(map: &serde_yaml::Mapping) -> BTreeMap<String, serde_yaml::Value> {
    map.iter()
        .filter_map(|(k, v)| k.as_str().map(|ks| (ks.to_string(), v.clone())))
        .collect()
}

/// Devuelve `Error::Validation` si el texto contiene patrones de secretos
/// (heurística simple para no incluirlos en prompts, RNF-05).
pub fn looks_like_secret_path(path: &str, patterns: &[String]) -> bool {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    for pat in patterns {
        if glob_match(&pat.to_lowercase(), &name) {
            return true;
        }
    }
    false
}

/// Coincidencia glob mínima: `*` en cualquier posición, `?`, sin directorios.
///
/// Iterativa, con retroceso solo hasta el último `*`: tiempo O(n·m) en el
/// peor caso, sin la explosión exponencial de la versión recursiva ante
/// patrones como `*a*a*a*…b` (vienen del TOML del repositorio).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Posición del último `*` visto y del texto cuando se vio.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || (p[pi] != '*' && p[pi] == t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            // El `*` absorbe un carácter más y se reintenta.
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_field_keeps_bom_crlf_and_body() {
        let text = "\u{feff}---\r\nid: x\r\nstatus: draft\r\n---\r\nstatus: no es frontmatter\r\n\r\n---\r\n";
        let out = set_field(text, "status", "stale");
        assert_eq!(
            out,
            "\u{feff}---\r\nid: x\r\nstatus: stale\r\n---\r\nstatus: no es frontmatter\r\n\r\n---\r\n"
        );
        let added = set_field("---\nid: x\n---\n# T\n---\n", "status", "stale");
        assert_eq!(added, "---\nid: x\nstatus: stale\n---\n# T\n---\n");
        assert_eq!(
            set_field("# sin frontmatter\n", "status", "x"),
            "# sin frontmatter\n"
        );
    }

    #[test]
    fn counts_lines_like_the_validator() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a"), 1);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b"a\r\nb\r\n"), 2);
        assert_eq!(count_lines(b"a\r\nb"), 2);
        assert_eq!(count_lines(b"\n"), 1);
        assert_eq!(count_lines(b"\n\n"), 2);
    }

    #[test]
    fn clamps_only_ranges_that_run_past_the_end() {
        // Rango de archivo completo redondeado hacia arriba.
        assert_eq!(
            clamp_source("src/views/form.ejs:1-400", 339).as_deref(),
            Some("src/views/form.ejs:1-339")
        );
        // Inicio igual a la última línea: queda una sola línea.
        assert_eq!(clamp_source("a.ejs:14-50", 14).as_deref(), Some("a.ejs:14"));
        // Válidos, irreparables o sin rango: no se tocan.
        assert_eq!(clamp_source("a.ejs:1-14", 14), None);
        assert_eq!(clamp_source("a.ejs:20-50", 14), None);
        assert_eq!(clamp_source("a.ejs:20", 14), None);
        assert_eq!(clamp_source("a.ejs:0-50", 14), None);
        assert_eq!(clamp_source("a.ejs:9-5", 14), None);
        assert_eq!(clamp_source("a.ejs", 14), None);
        // Archivo vacío: cualquier rango sigue siendo un error.
        assert_eq!(clamp_source("vacio.txt:1-5", 0), None);
    }

    #[test]
    fn replace_sources_is_a_minimal_edit() {
        let text = "---\r\nid: m\r\ntype: module\r\n# comentario\r\nsources:\r\n  - src/a.ejs:1-400 # todo el archivo\r\n  - \"src/b.ejs:1-40\"\r\n  - src/a.ejs:1-4000\r\ntitle: Vistas\r\n---\r\n# Cuerpo src/a.ejs:1-400\r\n";
        let changes = vec![
            ("src/a.ejs:1-400".to_string(), "src/a.ejs:1-339".to_string()),
            ("src/b.ejs:1-40".to_string(), "src/b.ejs:1-14".to_string()),
        ];
        let out = replace_sources(text, &changes).unwrap();
        assert_eq!(
            out,
            "---\r\nid: m\r\ntype: module\r\n# comentario\r\nsources:\r\n  - src/a.ejs:1-339 # todo el archivo\r\n  - \"src/b.ejs:1-14\"\r\n  - src/a.ejs:1-4000\r\ntitle: Vistas\r\n---\r\n# Cuerpo src/a.ejs:1-400\r\n"
        );
        // Lista en línea.
        let flow = "---\nid: m\nsources: [a.rs:1-9, 'b.rs:3-8']\ntype: module\n---\n";
        let out = replace_sources(flow, &[("b.rs:3-8".into(), "b.rs:3".into())]).unwrap();
        assert_eq!(
            out,
            "---\nid: m\nsources: [a.rs:1-9, 'b.rs:3']\ntype: module\n---\n"
        );
        // Sin `sources:` no hay nada que editar.
        assert!(replace_sources("---\nid: m\n---\n", &changes).is_none());
    }

    #[test]
    fn parses_frontmatter() {
        let text = "---\nid: f-1\ntype: feature\nsources:\n  - a.rs:1\n---\n# body";
        let fm = NoteFrontmatter::parse(text).unwrap();
        assert_eq!(fm.id.as_deref(), Some("f-1"));
        assert_eq!(fm.note_type.as_deref(), Some("feature"));
        assert_eq!(fm.sources, vec!["a.rs:1".to_string()]);
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*.pem", "cert.pem"));
        assert!(glob_match(".env*", ".env.local"));
        assert!(!glob_match("*.key", "keystore.toml"));
        assert!(glob_match("*id_rsa*", "my_id_rsa.pub"));
        assert!(glob_match("a?c", "abc") && !glob_match("a?c", "ac"));
        assert!(glob_match("*", "") && !glob_match("?", ""));
        // Patrón patológico: debe terminar al instante.
        let pat = format!("{}b", "*a".repeat(30));
        let text = "a".repeat(60);
        let start = std::time::Instant::now();
        assert!(!glob_match(&pat, &text));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn evidence_outside_project_is_reported() {
        let root = std::env::temp_dir().join(format!("adoc-ev-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.rs"), "x\n").unwrap();
        for src in ["../../etc/passwd:1", "/etc/passwd"] {
            let text = format!("---\nid: n\ntype: module\nsources:\n  - {src}\n---\n");
            let issues = validate_note(Path::new("n.md"), &text, &root);
            assert!(
                issues
                    .iter()
                    .any(|i| i.message == t!("core.note.source_outside", source = src)),
                "{src}: {issues:?}"
            );
        }
        let ok = "---\nid: n\ntype: module\nsources:\n  - ./a.rs:1\n---\n";
        assert!(validate_note(Path::new("n.md"), ok, &root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
