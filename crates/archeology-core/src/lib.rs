//! archeology-core: modelos, configuración, estado, plantillas y validación.
//!
//! Este crate contiene la lógica compartida por el resto de componentes:
//! - [`config`]: carga y guardado de `archeology-doc.toml`.
//! - [`context`]: AGENTS.md, adaptadores por herramienta y skills.
//! - [`fsutil`]: escritura segura (sin symlinks, atómica) de salidas generadas.
//! - [`gitcmd`]: invocación de `git` sin fsmonitor ni hooks del repositorio.
//! - [`i18n`]: idiomas (`en`, `es`), catálogos y macros [`t!`] / [`tl!`].
//! - [`paths`]: directorio de configuración y espacios de trabajo globales.
//! - [`project`]: descubrimiento del proyecto inspeccionado y sus rutas.
//! - [`state`]: estado incremental (`state.json` del espacio de trabajo global).
//! - [`model`]: estructuras de datos del inventario y tareas.
//! - [`frontmatter`]: parsing y validación del frontmatter de las notas.
//! - [`templates`]: plantillas embebidas de notas, tareas y sitio.
//! - [`graph`]: grafo del conocimiento (notas, relaciones y evidencias).
//! - [`graph_web`]: grafo en HTML (página independiente, reporte, sitio).
//! - [`index`]: índice fuente → notas y marcado de notas `stale`.
//! - [`managed`]: regiones gestionadas en archivos compartidos (p. ej. `AGENTS.md`).
//! - [`site`]: preparación del sitio VitePress y de `llms.txt`.
//! - [`planner`]: división del proyecto en tareas acotadas.
//! - [`validate`]: validaciones del comando `check`.
//! - [`task_context`]: re-verificación y retroalimentación en los archivos de tarea.
//! - [`trust`]: aprobación de los adaptadores definidos en el repositorio.
//! - [`zone`]: contexto documentado de una zona del código (`adoc context`).

pub mod config;
pub mod context;
pub mod error;
pub mod frontmatter;
pub mod fsutil;
pub mod gitcmd;
pub mod graph;
pub mod graph_web;
pub mod i18n;
pub mod index;
pub mod managed;
pub mod model;
pub mod paths;
pub mod planner;
pub mod project;
pub mod site;
pub mod state;
pub mod task_context;
pub mod templates;
pub mod trust;
pub mod validate;
pub mod zone;

pub use config::ArcheologyConfig;
pub use error::{Error, Result};
pub use i18n::Lang;
pub use project::Project;
