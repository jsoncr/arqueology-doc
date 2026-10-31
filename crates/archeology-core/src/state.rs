//! Estado incremental en `state.json` del espacio de trabajo global del
//! proyecto (`~/.archeology-doc/projects/<nombre>-<id8>/`, RF-120, RF-123).

use crate::error::Result;
use crate::model::RunRecord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    /// Hash blake3 por archivo fuente (ruta relativa POSIX → hash).
    #[serde(default)]
    pub file_hashes: BTreeMap<String, String>,
    /// Estado de tareas (task_id → status).
    #[serde(default)]
    pub tasks: BTreeMap<String, String>,
    /// Mapeo archivo fuente → notas que lo citan como evidencia.
    #[serde(default)]
    pub source_to_notes: BTreeMap<String, Vec<String>>,
    /// Notas marcadas como desactualizadas (RF-121).
    #[serde(default)]
    pub stale_notes: Vec<String>,
    /// Por qué está desactualizada cada nota: fuentes cambiadas que la
    /// volvieron `stale` (nota → fuentes). Se limpia al re-verificarla.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stale_reasons: BTreeMap<String, Vec<String>>,
    /// Historial de ejecuciones.
    #[serde(default)]
    pub runs: Vec<RunRecord>,
    /// Preguntas pendientes del agente por tarea (estado `question`).
    #[serde(default)]
    pub questions: BTreeMap<String, String>,
    /// ID completo del proyecto dueño de este espacio de trabajo: la carpeta
    /// se localiza por un prefijo corto y esto confirma la coincidencia.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Raíz del proyecto que usó este estado por última vez. Si cambia, otro
    /// clon o worktree con el mismo `project.id` comparte el estado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Commit de git en el que `sync` comparó por última vez las fuentes con
    /// las notas (`adoc context` avisa si cambió código desde entonces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_commit: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            file_hashes: BTreeMap::new(),
            tasks: BTreeMap::new(),
            source_to_notes: BTreeMap::new(),
            stale_notes: Vec::new(),
            stale_reasons: BTreeMap::new(),
            runs: Vec::new(),
            questions: BTreeMap::new(),
            project_id: None,
            project_root: None,
            knowledge_commit: None,
        }
    }
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        let mut state: Self = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                // Copia anterior a la última escritura (ver `save`).
                let bak = path.with_extension("json.bak");
                let recovered = fs::read_to_string(&bak)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .ok_or(e)?;
                eprintln!(
                    "{}",
                    crate::t!(
                        "core.state.recovered_backup",
                        path = path.display(),
                        backup = bak.display()
                    )
                );
                recovered
            }
        };
        if state.version == 0 {
            state.version = 1;
        }
        // Claves de fuentes normalizadas (`./a.rs`, `a\b.rs` de versiones
        // anteriores) para que coincidan con las rutas de git.
        state.file_hashes = std::mem::take(&mut state.file_hashes)
            .into_iter()
            .map(|(k, v)| (crate::index::normalize_rel_path(&k), v))
            .collect();
        let index = std::mem::take(&mut state.source_to_notes);
        for (k, notes) in index {
            let entry = state
                .source_to_notes
                .entry(crate::index::normalize_rel_path(&k))
                .or_default();
            for n in notes {
                if !entry.contains(&n) {
                    entry.push(n);
                }
            }
        }
        Ok(state)
    }

    /// Escritura atómica: temporal con nombre único + fsync + rename. Antes
    /// se guarda la versión anterior en `state.json.bak` para recuperarla.
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let tmp = path.with_extension(format!("json.{}-{nanos}.tmp", std::process::id()));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            f.sync_all()?;
        }
        if path.is_file() {
            let _ = fs::copy(path, path.with_extension("json.bak"));
        }
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Fija la lista de notas desactualizadas y descarta los motivos de las
    /// que ya no lo están (re-verificadas o corregidas).
    pub fn set_stale_notes(&mut self, notes: Vec<String>) {
        self.stale_reasons.retain(|n, _| notes.contains(n));
        self.stale_notes = notes;
    }

    /// Registra que `source` cambió y volvió `stale` a `note`.
    pub fn add_stale_reason(&mut self, note: &str, source: &str) {
        let list = self.stale_reasons.entry(note.to_string()).or_default();
        if !list.iter().any(|s| s == source) {
            list.push(source.to_string());
        }
    }

    /// Notas afectadas por una lista de fuentes cambiadas. Las rutas se
    /// normalizan (`./`, `\`) antes de buscarlas en el índice.
    pub fn affected_notes(&self, changed: &[String]) -> Vec<String> {
        let mut notes = Vec::new();
        for src in changed {
            let key = crate::index::normalize_rel_path(src);
            if let Some(list) = self.source_to_notes.get(&key) {
                for n in list {
                    if !notes.contains(n) {
                        notes.push(n.clone());
                    }
                }
            }
        }
        notes
    }
}

/// Hash blake3 de un archivo, en hex.
///
/// Por streaming: no carga el archivo entero en memoria. Solo archivos
/// regulares (no `/dev/zero`, FIFOs ni directorios).
pub fn file_hash(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            crate::t!("core.state.not_regular_file"),
        ));
    }
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affected_notes_match_normalized_paths() {
        let mut state = State::default();
        state
            .source_to_notes
            .insert("src/a.rs".into(), vec!["knowledge/modules/a.md".into()]);
        assert_eq!(
            state.affected_notes(&["./src/a.rs".into()]),
            vec!["knowledge/modules/a.md"]
        );
        assert_eq!(
            state.affected_notes(&["src\\a.rs".into()]),
            vec!["knowledge/modules/a.md"]
        );
    }

    #[test]
    fn load_normalizes_legacy_keys() {
        let dir = std::env::temp_dir().join(format!("adoc-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"file_hashes":{"./src/a.rs":"h"},"source_to_notes":{"./src/a.rs":["n.md"]}}"#,
        )
        .unwrap();
        let state = State::load(&path).unwrap();
        assert_eq!(
            state.file_hashes.get("src/a.rs").map(String::as_str),
            Some("h")
        );
        assert!(state.source_to_notes.contains_key("src/a.rs"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
