//! Índice fuente → notas y marcado de notas desactualizadas (RF-120 a RF-123).
//!
//! El índice se reconstruye siempre desde el frontmatter (`sources`) de las
//! notas del vault: las notas son la fuente de verdad, el estado solo cachea.

use crate::error::Result;
use crate::frontmatter::NoteFrontmatter;
use crate::project::Project;
use crate::state::State;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Parte de archivo de una evidencia `ruta:línea` o `ruta:a-b`.
pub fn source_file(src: &str) -> &str {
    match src.rsplit_once(':') {
        Some((file, lines))
            if !lines.is_empty() && lines.chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            file
        }
        _ => src,
    }
}

/// Rango de líneas de una evidencia, si lo tiene: `a.rs:10` → (10, 10).
pub fn source_lines(src: &str) -> Option<(usize, usize)> {
    let (_, lines) = src.rsplit_once(':')?;
    let (a, b) = match lines.split_once('-') {
        Some((a, b)) => (a.parse().ok()?, b.parse().ok()?),
        None => {
            let n = lines.parse().ok()?;
            (n, n)
        }
    };
    Some((a, b))
}

/// Normaliza una ruta relativa de fuente sin conocer la raíz: `\` → `/`,
/// sin `./` iniciales ni segmentos `.` intermedios y sin `/` duplicadas.
/// Así `./src/a.rs`, `src\a.rs` y `src//a.rs` son la misma clave que la
/// ruta que devuelve git (`src/a.rs`).
pub fn normalize_rel_path(path: &str) -> String {
    let unified = path.trim().replace('\\', "/");
    let absolute = unified.starts_with('/');
    let parts: Vec<&str> = unified
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Normaliza la parte de archivo de una fuente (`sources`) a una ruta
/// relativa a `root` con `/`: quita `./`, convierte `\` y, si es absoluta
/// dentro del proyecto, la hace relativa. Es la clave de `source_to_notes`
/// y `file_hashes`, comparable con las rutas de `git diff` (`sync --since`).
pub fn normalize_source(root: &Path, src: &str) -> String {
    let rel = normalize_rel_path(src);
    let is_abs = rel.starts_with('/') || Path::new(src.trim()).is_absolute();
    if is_abs {
        let root_text = normalize_rel_path(&root.to_string_lossy());
        if let Some(rest) = rel.strip_prefix(&format!("{root_text}/")) {
            return rest.to_string();
        }
        // Raíz con otra forma (symlinks, `\\?\`): se prueba la canónica.
        if let Ok(real) = crate::paths::canonicalize(root) {
            let real_text = normalize_rel_path(&real.to_string_lossy());
            if let Some(rest) = rel.strip_prefix(&format!("{real_text}/")) {
                return rest.to_string();
            }
        }
    }
    rel
}

/// Notas `.md` del vault (sin `_templates/`), en rutas absolutas.
pub fn vault_notes(project: &Project) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&project.knowledge_dir(), &mut out);
    out.sort();
    out
}

/// Nota del vault leída y parseada una sola vez (la comparten el grafo y
/// `adoc context`).
#[derive(Debug, Clone)]
pub struct VaultNote {
    /// Ruta absoluta.
    pub file: PathBuf,
    /// Ruta relativa a la raíz del proyecto (`knowledge/modules/a.md`).
    pub rel: String,
    /// Ruta relativa al vault sin `.md` (`modules/a`): id de nodo del grafo.
    pub id: String,
    /// Texto completo.
    pub text: String,
    pub fm: Option<NoteFrontmatter>,
}

impl VaultNote {
    /// Último segmento del id (`modules/a` → `a`).
    pub fn stem(&self) -> &str {
        self.id.rsplit('/').next().unwrap_or(&self.id)
    }

    /// Tipo de nota del frontmatter (`unknown` si falta).
    pub fn kind(&self) -> &str {
        self.fm
            .as_ref()
            .and_then(|f| f.note_type.as_deref())
            .unwrap_or("unknown")
    }
}

/// Lee y parsea todas las notas del vault (orden por ruta). Las ilegibles
/// se omiten.
pub fn load_vault(project: &Project) -> Vec<VaultNote> {
    let knowledge = project.knowledge_dir();
    vault_notes(project)
        .into_iter()
        .filter_map(|file| {
            let text = fs::read_to_string(&file).ok()?;
            let id = file
                .strip_prefix(&knowledge)
                .ok()?
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            Some(VaultNote {
                rel: project.rel(&file),
                fm: NoteFrontmatter::parse(&text),
                id,
                text,
                file,
            })
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        // Sin seguir symlinks: evita ciclos y leer fuera del árbol.
        let kind = e.file_type().ok();
        if kind.is_some_and(|t| t.is_dir()) {
            if p.file_name().map(|n| n == "_templates").unwrap_or(false) {
                continue;
            }
            collect(&p, out);
        } else if kind.is_some_and(|t| t.is_file())
            && p.extension().map(|x| x == "md").unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// Mapa archivo fuente → notas que lo citan (rutas relativas a la raíz).
pub fn source_index(project: &Project) -> BTreeMap<String, Vec<String>> {
    let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in vault_notes(project) {
        let Ok(text) = fs::read_to_string(&note) else {
            continue;
        };
        let Some(fm) = NoteFrontmatter::parse(&text) else {
            continue;
        };
        let rel = project.rel(&note);
        for src in &fm.sources {
            let file = normalize_source(&project.root, source_file(src));
            if file.is_empty() {
                continue;
            }
            let notes = index.entry(file).or_default();
            if !notes.contains(&rel) {
                notes.push(rel.clone());
            }
        }
    }
    index
}

/// Actualiza el índice y registra el hash actual de las fuentes citadas por
/// las notas modificadas desde `since`.
///
/// Se llama cuando una nota se (re)genera: a partir de ahí, un cambio en sus
/// fuentes la volverá `stale`.
pub fn refresh_hashes(project: &Project, state: &mut State, since: SystemTime) {
    refresh_where(project, state, |note| {
        fs::metadata(note)
            .and_then(|m| m.modified())
            .map(|m| m >= since)
            .unwrap_or(false)
    });
}

/// Igual que [`refresh_hashes`], para las notas cubiertas por las rutas
/// `produce` de una tarea (directorios terminados en `/` o archivos).
pub fn refresh_hashes_for(project: &Project, state: &mut State, produce: &[String]) {
    let rels: Vec<String> = produce.to_vec();
    refresh_where(project, state, |note| {
        let rel = project.rel(note);
        rels.iter()
            .any(|p| rel == *p || (p.ends_with('/') && rel.starts_with(p.as_str())))
    });
}

fn refresh_where(project: &Project, state: &mut State, include: impl Fn(&Path) -> bool) {
    state.source_to_notes = source_index(project);
    for note in vault_notes(project) {
        if !include(&note) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&note) else {
            continue;
        };
        let Some(fm) = NoteFrontmatter::parse(&text) else {
            continue;
        };
        for src in &fm.sources {
            let file = normalize_source(&project.root, source_file(src));
            if file.is_empty() {
                continue;
            }
            if let Some(h) = project.source_hash(&file) {
                state.file_hashes.insert(file, h);
            }
        }
    }
    state.set_stale_notes(stale_notes(project));
}

/// Notas cuyo frontmatter tiene `status: stale` (o `stale: true`).
pub fn stale_notes(project: &Project) -> Vec<String> {
    vault_notes(project)
        .into_iter()
        .filter(|p| {
            fs::read_to_string(p)
                .ok()
                .and_then(|t| NoteFrontmatter::parse(&t))
                .map(|fm| crate::validate::is_stale(&fm))
                .unwrap_or(false)
        })
        .map(|p| project.rel(&p))
        .collect()
}

/// Carpetas del vault con nota índice (MOC). El título es la clave
/// `doc.moc.<carpeta>` en el idioma de la documentación del proyecto.
const MOC_DIRS: &[&str] = &[
    "modules",
    "features",
    "findings",
    "tests",
    "specs",
    // El glosario es una nota por término; el planner no produce
    // `glossary/index.md`, así que este índice no choca con ninguna tarea.
    "glossary",
    "decisions",
    "faq",
];

/// Nombre de archivo de las notas índice.
pub const MOC_FILE: &str = "index.md";

/// Escribe `<carpeta>/index.md` con enlaces a las notas de cada tipo
/// (RF-064). La lista vive en una región gestionada; el resto es del usuario.
/// Devuelve las notas índice modificadas.
pub fn write_mocs(project: &Project) -> Result<Vec<String>> {
    let knowledge = project.knowledge_dir();
    let wikilinks = project.config.links.style != "relative";
    let lang = project.doc_lang();
    let mut changed = Vec::new();
    for dir in MOC_DIRS {
        let title = crate::tl!(lang, &format!("doc.moc.{dir}"));
        let folder = knowledge.join(dir);
        let mut notes: Vec<(String, String, Option<String>)> = Vec::new();
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        // Solo archivos regulares: un symlink podría apuntar fuera del vault.
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .collect();
        paths.sort();
        for p in paths {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if !name.ends_with(".md") || name == MOC_FILE {
                continue;
            }
            let fm = fs::read_to_string(&p)
                .ok()
                .and_then(|t| NoteFrontmatter::parse(&t));
            let stem = name.trim_end_matches(".md").to_string();
            let label = fm
                .as_ref()
                .and_then(|f| f.title.clone())
                .unwrap_or_else(|| stem.clone());
            // Hallazgos: severidad y estado junto al enlace.
            let extra = fm.as_ref().and_then(|f| {
                let sev = f.get("severity")?.as_str()?;
                let st = f.get("state").and_then(|v| v.as_str()).unwrap_or("open");
                Some(format!("{sev} · {st}"))
            });
            notes.push((stem, label, extra));
        }
        if notes.is_empty() {
            continue;
        }
        let mut body = String::new();
        for (stem, label, extra) in &notes {
            let link = if wikilinks {
                format!("[[{dir}/{stem}|{label}]]")
            } else {
                format!("[{label}](./{stem}.md)")
            };
            match extra {
                Some(e) => body.push_str(&format!("- {link} — {e}\n")),
                None => body.push_str(&format!("- {link}\n")),
            }
        }
        let path = folder.join(MOC_FILE);
        crate::fsutil::check_no_symlinks(&project.root, &path)?;
        let existing = crate::managed::read_or_default(&path, || {
            format!(
                "---\nid: moc-{dir}\ntype: moc\ntitle: {title}\nstatus: draft\n---\n\n# {title}\n"
            )
        })?;
        if NoteFrontmatter::parse(&existing)
            .map(|f| f.locked)
            .unwrap_or(false)
        {
            continue;
        }
        let updated = crate::managed::upsert_region(&existing, "index", body.trim_end())?;
        if crate::fsutil::write_if_changed_in_root(&project.root, &path, &updated)? {
            changed.push(project.rel(&path));
        }
    }
    Ok(changed)
}

/// Resultado de marcar una nota como desactualizada.
#[derive(Debug, PartialEq, Eq)]
pub enum MarkResult {
    Marked,
    /// `locked: true`: archeology-doc no la modifica (RF-123).
    Locked,
    AlreadyStale,
    NoFrontmatter,
}

/// Pone `status: stale` en el frontmatter de la nota, salvo que esté bloqueada.
///
/// No sigue symlinks y escribe de forma atómica.
pub fn mark_stale(path: &Path) -> Result<MarkResult> {
    let Some(text) = crate::fsutil::read_to_string_nofollow(path)? else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            crate::t!("core.index.not_found", path = path.display()),
        )
        .into());
    };
    let Some(fm) = NoteFrontmatter::parse(&text) else {
        return Ok(MarkResult::NoFrontmatter);
    };
    if fm.locked || fm.status.as_deref() == Some("locked") {
        return Ok(MarkResult::Locked);
    }
    if fm.status.as_deref() == Some("stale") {
        return Ok(MarkResult::AlreadyStale);
    }
    crate::fsutil::atomic_write(
        path,
        set_frontmatter_field(&text, "status", "stale").as_bytes(),
    )?;
    Ok(MarkResult::Marked)
}

/// Reemplaza (o añade) un campo escalar del frontmatter sin tocar el resto.
pub fn set_frontmatter_field(text: &str, key: &str, value: &str) -> String {
    crate::frontmatter::set_field(text, key, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_source_references() {
        assert_eq!(source_file("src/a.rs:10-20"), "src/a.rs");
        assert_eq!(source_file("src/a.rs"), "src/a.rs");
        assert_eq!(source_file("C:/x.rs"), "C:/x.rs");
        assert_eq!(source_lines("a.rs:10-20"), Some((10, 20)));
        assert_eq!(source_lines("a.rs:7"), Some((7, 7)));
        assert_eq!(source_lines("a.rs"), None);
    }

    #[test]
    fn normalizes_source_keys() {
        let root = Path::new("/repo");
        assert_eq!(normalize_source(root, "./src/a.rs"), "src/a.rs");
        assert_eq!(normalize_source(root, "src\\api\\b.rs"), "src/api/b.rs");
        assert_eq!(normalize_source(root, "src//./c.rs"), "src/c.rs");
        assert_eq!(normalize_source(root, "/repo/src/d.rs"), "src/d.rs");
        assert_eq!(normalize_source(root, "/otro/e.rs"), "/otro/e.rs");
        assert_eq!(normalize_rel_path(".\\x.rs"), "x.rs");
    }

    #[test]
    fn sets_frontmatter_field_only_inside_frontmatter() {
        let text = "---\nid: a\nstatus: draft\n---\nstatus: body\n";
        let out = set_frontmatter_field(text, "status", "stale");
        assert_eq!(out, "---\nid: a\nstatus: stale\n---\nstatus: body\n");
        let added = set_frontmatter_field("---\nid: a\n---\n", "status", "stale");
        assert_eq!(added, "---\nid: a\nstatus: stale\n---\n");
    }
}
