//! Grafo del conocimiento: notas del vault y sus relaciones (como el grafo de
//! Obsidian), más los archivos de código citados como evidencia.
//!
//! Aristas:
//! - `link`: wikilinks `[[destino]]` del cuerpo y del frontmatter.
//! - `related`: campo `related: [id, ...]` (hallazgos relacionados).
//! - `module`: campo `modules: [nombre]` → nota `modules/<nombre>.md`.
//! - `tests`: un `test-report` con `feature: <id>` → la funcionalidad.
//! - `evidence`: `sources: [ruta:línea]` → nodo de código (`kind: code`).

use crate::frontmatter::NoteFrontmatter;
use crate::index::{load_vault, source_file, source_lines, VaultNote};
use crate::project::Project;
use crate::validate::extract_wikilinks;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct Node {
    /// Ruta relativa al vault sin `.md` (`modules/billing`) o `code:<ruta>`.
    pub id: String,
    pub title: String,
    /// Tipo de nota (`feature`, `finding`…) o `code`.
    pub kind: String,
    pub status: Option<String>,
    /// Severidad (solo hallazgos).
    pub severity: Option<String>,
    /// Estado del hallazgo (open, resolved…).
    pub state: Option<String>,
    pub stale: bool,
    /// Archivo a abrir al hacer clic.
    pub file: PathBuf,
    /// Primera línea citada (nodos de código).
    pub line: Option<usize>,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

/// Construye el grafo completo del vault del proyecto.
pub fn build(project: &Project) -> Graph {
    build_from(project, &load_vault(project))
}

/// Construye el grafo a partir de notas ya leídas ([`load_vault`]).
pub fn build_from(project: &Project, notes: &[VaultNote]) -> Graph {
    struct Parsed<'a> {
        id: &'a str,
        fm: Option<&'a NoteFrontmatter>,
        text: &'a str,
        file: &'a PathBuf,
    }
    let parsed: Vec<Parsed> = notes
        .iter()
        .map(|n| Parsed {
            id: &n.id,
            fm: n.fm.as_ref(),
            text: &n.text,
            file: &n.file,
        })
        .collect();

    // Resolución de destinos: ruta relativa, nombre de archivo e `id` del frontmatter.
    let mut by_path: BTreeMap<String, String> = BTreeMap::new();
    let mut by_stem: BTreeMap<String, String> = BTreeMap::new();
    let mut by_note_id: BTreeMap<String, String> = BTreeMap::new();
    for p in &parsed {
        by_path.insert(p.id.to_string(), p.id.to_string());
        let stem = p.id.rsplit('/').next().unwrap_or(p.id).to_string();
        by_stem.entry(stem).or_insert_with(|| p.id.to_string());
        if let Some(note_id) = p.fm.and_then(|f| f.id.clone()) {
            by_note_id.insert(note_id, p.id.to_string());
        }
    }
    let resolve = |target: &str| -> Option<String> {
        let t = target
            .split(['|', '#'])
            .next()?
            .trim()
            .trim_end_matches(".md");
        by_path
            .get(t)
            .or_else(|| by_stem.get(t))
            .or_else(|| by_note_id.get(t))
            .cloned()
    };

    let mut nodes = Vec::new();
    let mut edges: BTreeSet<Edge> = BTreeSet::new();
    let mut code: BTreeMap<String, Option<usize>> = BTreeMap::new();
    let add = |source: &str, target: String, kind: &str, edges: &mut BTreeSet<Edge>| {
        if source != target {
            edges.insert(Edge {
                source: source.to_string(),
                target,
                kind: kind.to_string(),
            });
        }
    };

    for p in &parsed {
        let fm = p.fm;
        let get = |k: &str| {
            fm.and_then(|f| f.get(k))
                .and_then(|v| v.as_str())
                .map(String::from)
        };
        let list = |k: &str| -> Vec<String> {
            fm.and_then(|f| f.get(k))
                .and_then(|v| v.as_sequence())
                .map(|s| {
                    s.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let kind = fm
            .and_then(|f| f.note_type.clone())
            .unwrap_or_else(|| "unknown".into());
        nodes.push(Node {
            id: p.id.to_string(),
            title: fm
                .and_then(|f| f.title.clone())
                .unwrap_or_else(|| p.id.rsplit('/').next().unwrap_or(p.id).to_string()),
            kind: kind.clone(),
            status: fm.and_then(|f| f.status.clone()),
            severity: get("severity"),
            state: get("state"),
            // Mismo criterio que `index::stale_notes`.
            stale: fm.is_some_and(crate::validate::is_stale),
            file: p.file.clone(),
            line: None,
        });

        for target in extract_wikilinks(p.text) {
            if let Some(t) = resolve(&target) {
                add(p.id, t, "link", &mut edges);
            }
        }
        for r in list("related") {
            if let Some(t) = resolve(&r) {
                add(p.id, t, "related", &mut edges);
            }
        }
        for m in list("modules") {
            if let Some(t) = resolve(&format!("modules/{m}")) {
                add(p.id, t, "module", &mut edges);
            }
        }
        if kind == "test-report" {
            if let Some(t) = get("feature").and_then(|f| resolve(&f)) {
                add(p.id, t, "tests", &mut edges);
            }
        }
        if let Some(f) = fm {
            for src in &f.sources {
                let file = source_file(src).to_string();
                let line = source_lines(src).map(|(a, _)| a);
                code.entry(file.clone()).or_insert(line);
                add(p.id, format!("code:{file}"), "evidence", &mut edges);
            }
        }
    }

    for (file, line) in code {
        nodes.push(Node {
            id: format!("code:{file}"),
            title: file.clone(),
            kind: "code".into(),
            status: None,
            severity: None,
            state: None,
            stale: false,
            file: project.root.join(&file),
            line,
        });
    }

    Graph {
        nodes,
        edges: edges.into_iter().collect(),
    }
}

/// Notas más conectadas que muestra el resumen.
pub const TOP_CONNECTED: usize = 5;

/// Resumen del grafo: lo que muestran `review`, el reporte y el sitio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Notas del vault (todo nodo que no es código).
    pub notes: usize,
    /// Archivos de código citados como evidencia.
    pub code_files: usize,
    /// Total de aristas.
    pub edges: usize,
    /// Aristas por tipo (`link`, `related`, `module`, `tests`, `evidence`).
    pub edges_by_kind: BTreeMap<String, usize>,
    /// Notas sin ninguna arista (sin contar índices/MOCs), ordenadas por id.
    pub orphans: Vec<NodeRef>,
    /// Notas con más vecinos distintos (hasta [`TOP_CONNECTED`], sin índices).
    pub top_connected: Vec<Connected>,
}

/// Referencia breve a una nota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeRef {
    pub id: String,
    pub title: String,
    pub kind: String,
}

/// Nota y su número de conexiones (vecinos distintos, incluido el código).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Connected {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub connections: usize,
}

/// ¿Es un índice (MOC, `index.md`)? Enlaza a todo: no cuenta como huérfana
/// ni compite por "más conectada".
fn is_index(n: &Node) -> bool {
    let stem = n.id.rsplit('/').next().unwrap_or(&n.id);
    n.kind == "moc" || stem == "index" || stem == "_index"
}

/// Calcula el [`Summary`] de un grafo.
pub fn summary(graph: &Graph) -> Summary {
    let mut neighbors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut edges_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for e in &graph.edges {
        *edges_by_kind.entry(e.kind.clone()).or_insert(0) += 1;
        neighbors.entry(&e.source).or_default().insert(&e.target);
        neighbors.entry(&e.target).or_default().insert(&e.source);
    }
    let degree = |id: &str| neighbors.get(id).map(|s| s.len()).unwrap_or(0);
    let notes: Vec<&Node> = graph.nodes.iter().filter(|n| n.kind != "code").collect();

    let mut orphans: Vec<NodeRef> = notes
        .iter()
        .filter(|n| !is_index(n) && degree(&n.id) == 0)
        .map(|n| NodeRef {
            id: n.id.clone(),
            title: n.title.clone(),
            kind: n.kind.clone(),
        })
        .collect();
    orphans.sort_by(|a, b| a.id.cmp(&b.id));

    let mut top: Vec<Connected> = notes
        .iter()
        .filter(|n| !is_index(n) && degree(&n.id) > 0)
        .map(|n| Connected {
            id: n.id.clone(),
            title: n.title.clone(),
            kind: n.kind.clone(),
            connections: degree(&n.id),
        })
        .collect();
    // Más conexiones primero; a igualdad, por id (orden estable).
    top.sort_by(|a, b| b.connections.cmp(&a.connections).then(a.id.cmp(&b.id)));
    top.truncate(TOP_CONNECTED);

    Summary {
        notes: notes.len(),
        code_files: graph.nodes.len() - notes.len(),
        edges: graph.edges.len(),
        edges_by_kind,
        orphans,
        top_connected: top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn node(id: &str, kind: &str) -> Node {
        Node {
            id: id.into(),
            title: id.to_uppercase(),
            kind: kind.into(),
            status: None,
            severity: None,
            state: None,
            stale: false,
            file: PathBuf::from(format!("{id}.md")),
            line: None,
        }
    }

    fn edge(source: &str, target: &str, kind: &str) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            kind: kind.into(),
        }
    }

    #[test]
    fn summary_counts_orphans_and_top_connected() {
        let mut nodes = vec![
            node("modules/a", "module"),
            node("modules/b", "module"),
            node("features/c", "feature"),
            node("features/lonely", "feature"),
            node("findings/z", "finding"),
            node("index", "moc"),
            node("modules/index", "module"),
            node("code:src/a.rs", "code"),
        ];
        // Más notas conectadas que el tope: solo entran las TOP_CONNECTED primeras.
        for i in 0..6 {
            nodes.push(node(&format!("faq/q{i}"), "faq"));
        }
        let mut edges = vec![
            edge("features/c", "modules/a", "link"),
            // Dos aristas al mismo vecino cuentan como una conexión.
            edge("features/c", "modules/a", "module"),
            edge("features/c", "modules/b", "link"),
            edge("findings/z", "features/c", "related"),
            edge("features/c", "code:src/a.rs", "evidence"),
            edge("modules/a", "code:src/a.rs", "evidence"),
            // El índice enlaza a todo, pero no compite.
            edge("index", "modules/a", "link"),
            edge("index", "modules/b", "link"),
            edge("index", "features/c", "link"),
        ];
        for i in 0..6 {
            edges.push(edge(&format!("faq/q{i}"), "modules/b", "link"));
        }
        let g = Graph { nodes, edges };
        let s = summary(&g);
        assert_eq!(s.notes, 13);
        assert_eq!(s.code_files, 1);
        assert_eq!(s.edges, 15);
        assert_eq!(s.edges_by_kind["link"], 11);
        assert_eq!(s.edges_by_kind["evidence"], 2);
        assert_eq!(s.edges_by_kind["module"], 1);
        assert_eq!(s.edges_by_kind["related"], 1);
        // Huérfanas: sin aristas y sin contar índices (`moc` ni `*/index`).
        let orphans: Vec<&str> = s.orphans.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(orphans, vec!["features/lonely"]);
        // modules/b: c, index, 6 faq = 8; features/c: a, b, z, código, index = 5;
        // modules/a: c, código, index = 3; luego faq/q0 y faq/q1 (1 cada una).
        let top: Vec<(&str, usize)> = s
            .top_connected
            .iter()
            .map(|c| (c.id.as_str(), c.connections))
            .collect();
        assert_eq!(
            top,
            vec![
                ("modules/b", 8),
                ("features/c", 5),
                ("modules/a", 3),
                ("faq/q0", 1),
                ("faq/q1", 1)
            ]
        );
        assert_eq!(s.top_connected[0].title, "MODULES/B");
        assert!(s.top_connected.iter().all(|c| c.id != "index"));
    }

    #[test]
    fn summary_of_empty_graph() {
        let s = summary(&Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
        });
        assert_eq!((s.notes, s.code_files, s.edges), (0, 0, 0));
        assert!(s.orphans.is_empty() && s.top_connected.is_empty());
    }

    #[test]
    fn builds_links_relations_and_evidence() {
        let tmp = std::env::temp_dir().join(format!("arq-graph-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let know = tmp.join("knowledge");
        fs::create_dir_all(know.join("modules")).unwrap();
        fs::create_dir_all(know.join("features")).unwrap();
        fs::create_dir_all(know.join("findings")).unwrap();
        fs::create_dir_all(know.join("tests")).unwrap();
        fs::write(
            tmp.join("archeology-doc.toml"),
            toml::to_string(&crate::config::ArcheologyConfig::new("g", "es")).unwrap(),
        )
        .unwrap();
        fs::write(
            know.join("modules/billing.md"),
            "---\nid: mod-billing\ntype: module\ntitle: Billing\n---\n",
        )
        .unwrap();
        fs::write(
            know.join("features/discount.md"),
            "---\nid: feat-discount\ntype: feature\ntitle: Descuento\nmodules: [billing]\nsources: [app/billing.py:7]\n---\nVer [[billing|módulo]].\n",
        )
        .unwrap();
        fs::write(
            know.join("findings/limit.md"),
            "---\nid: find-limit\ntype: finding\ntitle: Tope\nkind: contradiction\nseverity: high\nstate: open\nrelated: [feat-discount]\nsources: [app/billing.py:7]\n---\n",
        )
        .unwrap();
        fs::write(
            know.join("tests/discount.md"),
            "---\nid: test-discount\ntype: test-report\nfeature: feat-discount\n---\n",
        )
        .unwrap();
        std::env::set_var("ARCHEOLOGY_DOC_CONFIG_DIR", tmp.join("cfg"));
        let project = Project::open(&tmp).unwrap();
        let g = build(&project);
        let has = |s: &str, t: &str, k: &str| {
            g.edges
                .iter()
                .any(|e| e.source == s && e.target == t && e.kind == k)
        };
        assert!(has("features/discount", "modules/billing", "link"));
        assert!(has("features/discount", "modules/billing", "module"));
        assert!(has("findings/limit", "features/discount", "related"));
        assert!(has("tests/discount", "features/discount", "tests"));
        assert!(has("findings/limit", "code:app/billing.py", "evidence"));
        let code = g.nodes.iter().find(|n| n.kind == "code").unwrap();
        assert_eq!(code.line, Some(7));
        let finding = g.nodes.iter().find(|n| n.kind == "finding").unwrap();
        assert_eq!(finding.severity.as_deref(), Some("high"));
        fs::remove_dir_all(&tmp).ok();
    }
}
