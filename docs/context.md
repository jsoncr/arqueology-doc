# `adoc context` — contrato (schema_version 1)

`adoc context` entrega, en una sola llamada, el contexto documentado de una o varias zonas del
código: módulos, funcionalidades, notas por relevancia, hallazgos, tests, dependencias, decisiones,
glosario y preguntas abiertas. Es para consumo programático (agentes, scripts, la software factory)
y no usa agentes ni red: solo lee `knowledge/`, el estado local (`state.json`, `inventory.json`,
tareas) y `git rev-parse HEAD`. No escribe nada.

- JSON Schema: [`schemas/context.v1.schema.json`](../schemas/context.v1.schema.json).
- Versión soportada por el binario: `adoc doctor` (línea `context`, y `context_schema_version` en
  `adoc doctor --format json`).

## Interfaz

```
adoc context <objetivo>... [opciones]
```

| Objetivo | Resolución |
|----------|------------|
| `src/billing/`, `src/billing/invoice.rs` | Ruta relativa a la raíz o al directorio actual (`../billing` desde `src/api`), o absoluta dentro del proyecto (también a través de symlinks). `.` y `..` se resuelven antes de comprobar que no sale del proyecto. La raíz misma (`.`, `/ruta/proj`, `/ruta/proj/`) es todo el proyecto. Se acepta `\` (Windows): se normaliza a `/`. Una ruta que ya no existe vale si alguna nota o módulo la cita. |
| `module:billing` | Nombre del módulo del inventario; si no, la nota (`modules/billing`, nombre de archivo, `id` `billing` o `mod-billing`); si no, título normalizado (sin mayúsculas ni acentos). |
| `feature:checkout` | Nota `features/checkout`, nombre de archivo, `id` (`checkout` o `feat-checkout`); si no, título normalizado. |
| `billing` (sin prefijo) | Se prueba como ruta, luego como módulo y luego como funcionalidad; `resolved_targets[].kind` dice cuál ganó. |

Si varias notas coinciden en la misma etapa, se incluyen todas y se añade un aviso (`ambiguo`).
Un objetivo que coincide con `scan.secrets_patterns` no se resuelve.

| Opción | Por defecto | Efecto |
|--------|-------------|--------|
| `--query "<texto>"` | — | Busca en títulos y contenido (sin mayúsculas ni acentos). Aparece en `resolved_targets` con `kind: "query"`. **Con objetivos**, solo añade notas (relevancia de texto). **Sin objetivos**, las mejores coincidencias (título primero, hasta `--max-notes`) hacen de zona: sus módulos y funcionalidades, tests, dependencias y `--depth` se calculan como si fueran objetivos. |
| `--depth <n>` | `1` | Saltos del grafo (mismas aristas que `adoc graph`: `link`, `related`, `module`, `tests`) para dependencias y notas relacionadas. `0` = solo el objetivo. |
| `--include <lista>` | todas | `modules,features,notes,findings,tests,dependencies,decisions,glossary`. Las secciones excluidas quedan como listas vacías. |
| `--max-notes <n>` | `20` | Máximo de `notes`, por relevancia (aviso si se recorta). |
| `--max-chars <n>` | `0` (sin límite) | Presupuesto total del `content` de las notas, en orden de relevancia. La nota que se corta termina en `…[truncated]`; esa y las siguientes llevan `truncated: true`. |
| `--content none\|excerpt\|full` | `excerpt` | `none`: `""`. `excerpt`: región gestionada `summary` o los primeros párrafos. `full`: cuerpo sin frontmatter ni marcadores de regiones. |
| `--format text\|json\|markdown` | `text` | `json` es el contrato. `--json` es un alias de `--format json` (combinarlo con `--format text`/`markdown` es un error de uso). |
| `--fail-on-stale` | — | Código 3 si alguna nota incluida está desactualizada. |
| `--all-findings` | — | Incluye hallazgos `resolved` y `dismissed` (por defecto, solo abiertos). |

### Códigos de salida

| Código | Significado |
|--------|-------------|
| 0 | Éxito (aunque algún objetivo quede en `unresolved_targets`). |
| 1 | Error de uso (también los errores de argumentos de clap en este comando) o proyecto no inicializado. |
| 2 | Ningún objetivo pudo resolverse (la salida JSON igual se imprime, con sugerencias en `warnings`). |
| 3 | Éxito, pero hay notas desactualizadas y se pasó `--fail-on-stale`. |

## JSON

Reglas del contrato:

- `schema_version` es un entero. Añadir campos opcionales **no** lo incrementa; cambiar o eliminar
  campos sí. Los consumidores deben ignorar los campos desconocidos.
- Las listas siempre están presentes (nunca `null`). Las rutas son relativas a la raíz, con `/`
  (`project.root` es la única absoluta).
- `relevance` va de 0 a 1 con dos decimales.
- Orden determinista: notas por relevancia y luego ruta; hallazgos por severidad, relevancia y ruta;
  dependencias por nivel, origen y destino.

| Campo | Origen |
|-------|--------|
| `project.knowledge_commit` | Commit guardado por `adoc sync` en `state.json` (`scan` no lo escribe porque no re-verifica las notas); si falta, el del último inventario. Si difiere de `head_commit`, una llamada a `git diff --name-only` cuenta los archivos de código cambiados entre ambos, sin el vault, `docs/`, los archivos de adoc (`AGENTS.md`, `CLAUDE.md`, `llms.txt`, `.agents/`, `.cursor/rules/archeology-doc.mdc`, `.github/copilot-instructions.md`, `archeology-doc.toml`) ni secretos: solo si queda alguno hay aviso. Si git no conoce ese commit, aviso más suave. |
| `modules[]` | Módulos resueltos (y los de las funcionalidades resueltas). `id` = `module:<nombre del inventario>` (o el nombre de archivo de la nota si el módulo no está en el inventario); `paths` del inventario (o las carpetas de las evidencias de la nota); `summary` = región `summary` o primer párrafo. |
| `features[]` | Funcionalidades resueltas, con evidencia en la zona o de un módulo resuelto. Los valores de `modules:` se resuelven con los mismos alias que `module:` (`mod-billing`, id, título) y se emiten con el nombre canónico (`module:billing`); si no corresponden a ningún módulo, el valor tal cual. `id` = `feature:<nombre de archivo>`. |
| `notes[]` | Notas relevantes (sin índices/MOCs) con `type`, `content`, `evidence` y estado. |
| `findings[]` | Notas `type: finding` relevantes. `category` sale de `kind`: `contradiction`→`contradiction`, `stale-doc`→`outdated_doc`, `duplication`→`duplicated_rule`, `config`→`config_conflict`, `dead-code`→`dead_code`, resto→`other` (el valor original va en `kind`). `status`: `open`/`confirmed`→`open`, `resolved`, `dismissed`. |
| `tests.covered[]`, `tests.missing[]` | Secciones "Escenarios cubiertos/faltantes" (o "Covered/Missing scenarios") de los reportes `test-report` de las funcionalidades de la zona, en tabla o lista. `tests` = referencias `ruta:línea` de la fila; si no hay, `test_files` del frontmatter. |
| `tests.disabled_or_empty[]` | Tests desactivados del inventario dentro de la zona. Fase 1: `line: null` y solo `reason: "disabled"`. |
| `dependencies` | Aristas del grafo desde (`outgoing`) y hacia (`incoming`) la zona, hasta `--depth`, con `kinds`. |
| `decisions[]`, `glossary[]` | Notas `decision` (sección "Decisión") y `glossary` (primer párrafo) relevantes. |
| `open_questions[]` | Preguntas de agentes (`state.json`, id de tarea) cuya tarea es de un módulo de la zona o produce una nota relevante, y dudas de `memory/open-questions.md` (`memory#N`) que mencionan un módulo, ruta o funcionalidad de la zona. |
| `warnings[]` | Textos en el idioma de la interfaz: sin inventario, commits distintos, objetivos sin resolver (con sugerencias), ambiguos, recortes, evidencias secretas omitidas, zona sin documentar, notas desactualizadas. |

### Relevancia

Cada nota toma el mayor valor de las reglas que cumple (bandas sin solaparse, en el orden de la
especificación):

| Regla | Valor |
|-------|-------|
| Coincidencia directa: nota del módulo o funcionalidad resuelto | 1.0 (0.9 para módulos deducidos de una ruta) |
| Evidencia en la ruta (o en las rutas del módulo / fuentes de la funcionalidad resueltos) | 0.75 + 0.14 × min(1, citas/3) |
| Dependencia de nivel 1 en el grafo | 0.6 |
| Coincidencia de `--query` | 0.35 (0.45 si es en el título) |
| Dependencia de nivel d ≥ 2 | 0.3 / d |

### `stale` y `evidence[].valid`

- `stale` usa el criterio de `adoc sync`: la nota está marcada (`status: stale` o `stale: true`;
  `stale_reason` lista las fuentes que cambiaron según `state.json`) **o** está pendiente: una fuente
  citada cambió desde el último `sync`/`scan` (su hash ya no coincide con el guardado), que es lo que
  `sync` marcaría. Las notas `locked` no quedan pendientes (sync no las marca). Solo se hashean las
  fuentes de las notas incluidas, nunca las secretas (no se leen); en `stale_reason` una fuente secreta
  se describe sin su ruta.
- `evidence[].valid` usa el criterio de `adoc check`: archivo dentro del proyecto, existente y con el
  rango de líneas dentro del archivo. `context` nunca repara evidencias (`check --fix` sí).

### Secretos

Las evidencias, objetivos, referencias a tests y motivos de `stale` que coinciden con
`scan.secrets_patterns` nunca se incluyen (ni se leen ni se hashean), aunque una nota los cite; un
aviso cuenta las evidencias omitidas (pares nota–evidencia únicos).

## Salidas de texto

- `text`: resumen para la terminal (zona, módulos, hallazgos por severidad, tests y escenarios sin
  cubrir, dependencias, notas, avisos y siguiente paso). Colores solo en una terminal.
- `markdown`: bloque para el prompt de un agente; avisos y notas desactualizadas al inicio, una
  sección por tipo y citas `ruta:línea`.

## Rendimiento

Cada llamada lee y parsea el vault una vez (O(notas + bytes del vault)), construye el grafo en
memoria y solo lee archivos de código para contar líneas de las evidencias incluidas. Con ~2000
notas tarda ~0,2 s (release) / ~0,5 s (debug); no hace falta un índice persistente.
