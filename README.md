# archeology-doc

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
  <img src="assets/logo.png" alt="archeology-doc" width="360">
</picture>

CLI escrito en Rust que inspecciona proyectos de software — nuevos o legacy, pequeños o grandes — y genera:

- una **base de conocimiento en Markdown** navegable con Obsidian (`knowledge/`),
- **documentación con VitePress** (`docs/`, manual de usuario, guías, referencia),
- **contexto y memoria para agentes de IA** (`AGENTS.md`, `knowledge/memory/`),
- **integración con OpenSpec**: cruza specs ↔ código y detecta desfases (en `review`, `status` y `check`;
  el comando `adoc spec` está desactivado temporalmente mientras se rediseña),
- un **registro global** de los proyectos en los que trabajas (`~/.archeology-doc/`).

La investigación la realizan los agentes que ya usas (Claude Code, Codex, OpenCode, Devin…); archeology-doc orquesta, estructura y valida.

## Instalación

Binario precompilado (sin Rust), desde el sitio publicado:

```bash
curl -fsSL https://archeology-doc.com/download/install.sh | sh          # macOS / Linux
irm https://archeology-doc.com/download/install.ps1 | iex                # Windows (PowerShell)
```

Desde el código (instaladores y build de descargas en [WEBSITE/](WEBSITE/README.md)):

```bash
WEBSITE/installers/install.sh  # compila este clon con cargo
cargo install --path crates/archeology-cli
# o desde el repo:
cargo build --release
# binarios en target/release/: `adoc` (corto) y `archeology-doc` (equivalente)

adoc doctor                    # verifica git, Node, VitePress, OpenSpec y agentes
adoc site setup                # opcional: VitePress compartido para todos los proyectos
```

Actualizar:

```bash
adoc upgrade --check           # solo informa: versión actual → última publicada (`--format json`)
adoc upgrade                   # descarga, verifica y sustituye `adoc` y `archeology-doc` (pide confirmación)
adoc upgrade --yes             # sin confirmación (obligatorio en scripts / sin terminal)
adoc upgrade --with-vscode     # además actualiza la extensión de VS Code (requiere `code`)
adoc upgrade --force           # reinstala aunque ya sea la última versión
```

`adoc upgrade` lee `latest.json` de la carpeta de descargas, compara con la versión en uso y, si hay
una nueva, baja `adoc-<target>.tar.gz` (`.zip` en Windows) de la plataforma, comprueba que su SHA-256
coincida con `latest.json` **y** con `SHA256SUMS`, y sustituye los dos binarios en la carpeta del
ejecutable en uso (renombrado atómico; en Windows el `.exe` en uso pasa a `adoc.exe.old` y se borra al
arrancar la próxima vez). Al terminar ejecuta `adoc --version` del binario nuevo. Si algo no verifica,
no se toca nada. `--check` sale siempre con código 0 haya o no versión nueva (solo falla si no se puede
consultar). La carpeta de descargas es `ARCHEOLOGY_DOC_DOWNLOAD` > `download_url` en
`~/.archeology-doc/config.toml` > `https://archeology-doc.com/download` (solo https; http únicamente
contra `localhost`/`127.0.0.1`).

- Instalado con `cargo install`: no se sustituye; `adoc upgrade` indica
  `cargo install --git https://github.com/jsoncr/archeology-doc archeology-cli --force`
  (o `cargo install --path crates/archeology-cli --force` desde un clon).
- Instalado con Homebrew: `brew upgrade archeology-doc`.
- Linux con musl u otra plataforma sin binarios publicados: compila desde el código.
- Sin permisos de escritura en la carpeta de los binarios: corrige los permisos o vuelve a ejecutar el
  instalador (`curl … | sh`, `irm … | iex`), que también sirve para actualizar desde versiones
  anteriores a `adoc upgrade`.

`adoc doctor` avisa si hay una versión nueva (consulta como mucho una vez al día, con 3 s de espera y
sin fallar sin red; resultado en `~/.archeology-doc/update-check.json`). Desactívalo con
`ADOC_NO_UPDATE_CHECK=1`.

Desinstalar:

```bash
adoc uninstall                 # quita los binarios `adoc` y `archeology-doc`
adoc uninstall --purge         # además borra ~/.archeology-doc (registro, estado, aprobaciones, VitePress)
```

Los proyectos no se tocan: `archeology-doc.toml`, `knowledge/`, `docs/` y las regiones de `AGENTS.md`
quedan en cada repositorio. Si se instaló con `cargo install`, usa `cargo uninstall archeology-cli`.

## Uso rápido

```bash
cd mi-proyecto-legacy

adoc init            # crea archeology-doc.toml + estructura
adoc scan            # inventario determinista → inventory.json + knowledge/00-overview.md
adoc plan            # genera tareas acotadas en ~/.archeology-doc/projects/<proyecto>/tasks/
adoc inspect         # ejecuta con el agente configurado (o exporta en modo manual)
adoc review          # resumen: hallazgos, tests, progreso
adoc status          # resumen del proyecto y siguiente paso sugerido
adoc context src/billing/   # contexto documentado de una zona (notas, hallazgos, tests, dependencias)
adoc check           # qué está hecho, qué falta y cómo arreglarlo (código de salida para CI)
```

Modo manual (agentes sin headless, p. ej. Devin/Cursor):

```bash
adoc inspect --agent manual     # exporta prompts
# …ejecuta cada tarea (`adoc task show T-001`) con tu agente…
adoc task done T-001            # marca completada tras revisar la salida
```

Trabajar desde otro directorio:

```bash
adoc projects list
adoc review --project mi-proyecto
adoc resume                     # retoma la inspección pendiente
```

## Dónde queda cada cosa

```
mi-proyecto/                          # solo configuración y resultados
├── archeology-doc.toml               # config + ID del proyecto (versionable)
├── knowledge/  docs/  llms.txt       # vault Obsidian, sitio VitePress
└── AGENTS.md  CLAUDE.md  .agents/    # contexto para agentes (regiones gestionadas)

~/.archeology-doc/                    # estado de trabajo, fuera del repositorio
├── registry.toml                     # índice de proyectos
├── vitepress/                        # VitePress compartido (`adoc site setup`)
└── projects/<nombre>-<id8>/          # por proyecto
    ├── state.json  inventory.json
    ├── tasks/                        # T-*.md
    └── runs/<fecha>-<id>/            # run.log y un log por tarea
```

Los proyectos con el estado antiguo dentro del repositorio (`<proyecto>/.archeology-doc/` o, con el
nombre anterior, `<proyecto>/.arqueology-doc/`) se migran solos al primer comando, y el registro de
`~/Library/Application Support/archeology-doc` o `~/Library/Application Support/arqueology-doc`
(o sus equivalentes en `~/.config/…`) también.

## Agentes

```bash
adoc agent list                       # disponibles; * = activo
adoc agent use claude --model opus    # cambia el agente y el modelo del proyecto
adoc agent use devin                  # Devin CLI
adoc agent use manual                 # sin agente: exporta tareas para hacerlas a mano
adoc agent use devin --reset          # vuelve a la definición predefinida del adaptador
adoc inspect --agent codex --model gpt-5   # solo para esta corrida
```

Agentes predefinidos (modo no interactivo con permiso de escritura):

| Agente | Comando |
|--------|---------|
| `claude` | `claude -p --permission-mode acceptEdits` (tarea por stdin) |
| `codex` | `codex exec -s workspace-write -` (stdin) |
| `devin` | `devin -p --permission-mode smart --respect-workspace-trust false --prompt-file <tarea>` |
| `gemini` | `gemini --approval-mode auto_edit` (stdin) |
| `qwen` | `qwen --approval-mode auto-edit` (stdin) |
| `opencode` | `opencode run <tarea>` |
| `cursor` | `agent -p --force --output-format text <tarea>` |
| `copilot` | `copilot -p <tarea> --allow-all-tools` |
| `amp` | `amp --dangerously-allow-all -x <tarea>` |
| `goose` | `goose run -i -` (stdin) |
| `aider` | `aider --message-file <tarea> --yes-always --no-auto-commits` |
| `manual` | exporta las tareas para hacerlas a mano |

Los flags de cada CLI cambian con el tiempo: `adoc agent test` y `adoc doctor` los comprueban, y cualquier
adaptador se puede redefinir en `[agent.adapters.<nombre>]`.

**Modelo.** `adoc agent use <agente> --model <modelo>` (o `--model` en `inspect`, `task run`, `resume`, `ask`)
pasa el modelo como lo espera cada CLI, antes del prompt; `agent use` y `agent test` muestran el comando
efectivo. Un modelo con espacios (el nombre visible, p. ej. "Claude Sonnet 4.5") se rechaza: usa el id.

| Agente | Cómo recibe el modelo | Ejemplo |
|--------|-----------------------|---------|
| `claude` | `--model <modelo>` (alias o nombre completo) | `sonnet`, `opus`, `claude-sonnet-4-5` |
| `codex` | `--model <modelo>` | `gpt-5`, `gpt-5-codex` |
| `devin` | `--model <modelo>` (`adoc agent models devin` los lista) | `opus`, `claude-sonnet-4` |
| `gemini` | `--model <modelo>` | `gemini-2.5-pro` |
| `qwen` | `--model <modelo>` | `qwen3-coder-plus` |
| `opencode` | `--model proveedor/modelo` (obligatorio el proveedor; `adoc agent models opencode` los lista) | `anthropic/claude-sonnet-4-5` |
| `cursor` | `--model <modelo>` | `sonnet-4.5`, `gpt-5` |
| `copilot` | `--model <modelo>` | `claude-sonnet-4.5`, `gpt-5` |
| `aider` | `--model <modelo>` (proveedor opcional) | `sonnet`, `anthropic/claude-sonnet-4-5` |
| `goose` | variables `GOOSE_PROVIDER`/`GOOSE_MODEL` (de `proveedor/modelo`) | `anthropic/claude-sonnet-4-5` |
| `amp` | no se puede elegir por línea de comandos (error si se fija) | — |

En un adaptador propio, el modelo se pasa con `{model}` en `args` (p. ej. `["--model", "{model}"]`);
sin `{model}` y con un CLI desconocido, fijar un modelo es un error en lugar de ignorarse.

**Límites del agente.** Si el agente se queda sin cuota, alcanza un límite de uso o de peticiones
(`usage limit`, `429`, `quota`, `RESOURCE_EXHAUSTED`, `credit balance is too low`…) o no tiene sesión
iniciada, `inspect` no reintenta: la tarea vuelve a `pending`, la corrida se detiene (no gasta las
siguientes tareas) y muestra el mensaje del agente, cuándo se restablece el límite (si lo dice) y qué
hacer: `adoc inspect --resume` tras el reinicio, otro agente (`adoc agent use codex` o
`--agent codex` solo para esa corrida) o cómo iniciar sesión en ese CLI. Código de salida **75**
(límite de uso o de peticiones, `EX_TEMPFAIL`) o **77** (sin sesión, `EX_NOPERM`). Si la tarea no
cabe en el contexto del modelo (`prompt is too long`, `context_length_exceeded`…), la tarea falla con
la sugerencia de dividirla (`[plan] max_files_per_task` / `max_bytes_per_task` y `adoc plan --force`)
y la corrida sigue con las demás. La clasificación queda en `run.log` y en el log de la tarea.

**Seguridad.** `archeology-doc.toml` viaja con el repositorio y puede venir de terceros, así que un
adaptador que no sea idéntico a uno predefinido no se ejecuta (ni siquiera `--version`) hasta que lo
revisas y lo apruebas:

```bash
adoc agent trust mi-agente     # muestra command, args y env, y pide confirmación
adoc agent untrust mi-agente   # retira la aprobación
```

La aprobación se guarda en `~/.archeology-doc/trusted-adapters.toml`, ligada al proyecto y a la
definición exacta: si el adaptador cambia en el repositorio, hay que volver a aprobarlo. Los
adaptadores `copilot`, `amp`, `aider`, `cursor` y `devin` (`--respect-workspace-trust false`) corren
sin confirmaciones (pueden ejecutar comandos de shell): `inspect` lo avisa. Úsalos solo con repositorios de confianza. `adoc preview` y
`adoc site` ejecutan la configuración de VitePress del proyecto (JavaScript): la primera vez en cada
proyecto piden confirmación (se recuerda en `trusted-adapters.toml`). Sin terminal interactiva, como en CI,
hay que aprobarlo con `--yes` (`adoc preview --yes`, `adoc site build --yes`, `adoc check --site --yes`).

`agent use` edita `archeology-doc.toml` conservando su formato. Con `manual`, `inspect` marca
las tareas como manuales; se ejecutan en esa herramienta y se cierran con `adoc task done <id>`.
Al cambiar a un agente con CLI, las tareas manuales sin completar vuelven a `pending` y basta `adoc inspect`;
las tareas `done` nunca se tocan.

## Comandos

| Comando | Descripción |
|---------|-------------|
| `init` | Inicializa config y estructura; si la carpeta ya está inicializada o registrada, o el nombre ya existe en otra ruta, lo explica sin sobrescribir (`--yes`, `--agent`, `--lang`, `--name`, `--adapters claude,cursor,copilot`, `--force`, `--new-id` para una carpeta copiada) |
| `scan` | Inventario determinista; actualiza `00-overview.md`, `AGENTS.md`, adaptadores, skills e índices (`--no-git`, `--json`) |
| `plan` | Genera/actualiza tareas priorizadas (`--module`, `--type`, `--path`, `--force`) |
| `inspect` | Ejecuta tareas con agente en orden de dependencias (`--agent`, `--model`, `--task`, `--module`, `--resume`, `--dry-run`) |
| `task list/show/export/done` | Gestión de tareas en modo manual (`done` valida la salida; `--force` la omite) |
| `task answer <id> "<texto>"` | Responde la pregunta de un agente y deja la tarea lista para reintentar |
| `task run <id>` | Ejecuta una sola tarea (`--interactive` abre la sesión del agente; `--agent`, `--model`) |
| `review` | Progreso, hallazgos, funcionalidades sin tests, cumplimiento de specs y resumen del grafo (`--severity`, `--format text\|json\|markdown`) |
| `check` | Lista de comprobación (✓ / ! / ✗ / –) de lo hecho y lo que falta, con cómo arreglarlo: configuración, inventario, tareas, overview, frontmatter, evidencias (archivo y líneas), wikilinks (con `archivo:línea`), duplicados, notas stale, AGENTS.md, sitio y OpenSpec. Código de salida para CI: falla con errores de validación (o advertencias con `--strict`) (`--strict`, `--format text\|json`, `--site`) |
| `sync` | Muestra qué fuentes documentadas cambiaron (M/A/D y notas afectadas), marca `stale` esas notas y reabre sus tareas; lista también los cambios que ninguna nota cubre desde el último `scan` (`--since <ref>`, `--dry-run`, `--format text\|json`) |
| `site setup` | Instala VitePress en el directorio de configuración, compartido por todos los proyectos (`--version`) |
| `site sync/dev/build` | Copia vault y specs a `docs/`, genera sidebar y `llms.txt`; `dev`/`build` delegan en VitePress |
| `status` | Resumen del proyecto en una pantalla: tipo, lenguajes, tamaño, git, estructura generada, progreso, tareas pendientes, preguntas, hallazgos, agente y el siguiente paso. `--format json`: estado completo (usado por la extensión) |
| `preview` | Sitio con dashboard en el navegador; sin VitePress, el reporte HTML (`--port`, `--no-open`) |
| `report` | Reporte HTML único o Markdown (`--format`, `--out`, `--no-open`) |
| `context <objetivo>...` | Contexto documentado de una o varias zonas del código en una llamada, para agentes y scripts: módulos, funcionalidades, notas por relevancia, hallazgos abiertos, escenarios cubiertos/faltantes, dependencias del grafo, decisiones, glosario y preguntas abiertas. Objetivos: ruta, `module:<nombre>`, `feature:<nombre>` o un nombre sin prefijo (`--query`, `--depth`, `--include`, `--max-notes`, `--max-chars`, `--content none\|excerpt\|full`, `--format text\|json\|markdown` o `--json`, `--fail-on-stale`, `--all-findings`). Códigos: 0 éxito, 1 uso/proyecto, 2 nada resuelto, 3 notas desactualizadas con `--fail-on-stale`. Contrato: [`docs/context.md`](docs/context.md) |
| `graph` | Grafo del conocimiento: genera el HTML interactivo y lo abre en el navegador; `--json` imprime el JSON (vista de la extensión) (`--out`, `--no-open`; `ADOC_NO_OPEN=1` evita el navegador) |
| `ask "<pregunta>"` | Pregunta al agente; la respuesta queda en la FAQ (`--agent`, `--model`) |
| `doctor` | Verifica requisitos del CLI y del proyecto; código 1 si falta algo obligatorio (`--format`) |
| `upgrade` | Actualiza `adoc` a la última versión publicada verificando SHA-256 (`--check`, `--format text\|json`, `--yes`, `--force`, `--with-vscode`) |
| `uninstall` | Quita los binarios; `--purge` borra también `~/.archeology-doc` (`--yes`) |
| `agent list/test` | Lista y prueba adaptadores; `*` marca el activo |
| `agent use <agente>` | Cambia el agente activo del proyecto, p. ej. `claude` o `devin` (`--model`, `--no-model`, `--reset`) |
| `agent models [agente]` | Cómo recibe el modelo el agente y, si su CLI sabe listarlos (`opencode`, `devin`), los disponibles |
| `projects list/add/discover/prune` | Registro global con progreso, hallazgos y cumplimiento (`--format`) |
| `projects delete <nombre\|id>` | Muestra qué se borrará y pide confirmación: entrada del registro, espacio de trabajo y aprobaciones; `-d` también los archivos generados en el proyecto (conserva el texto propio de AGENTS.md y los docs del usuario) (`--yes`, `-d`) |
| `resume [proyecto]` | Retoma la última inspección pendiente |

Opción global: `--project <nombre|id>` en todos los comandos.

## Cómo funciona una tarea

1. `plan` escribe las tareas `T-*.md` en el espacio de trabajo con objetivo, archivos a leer, salidas (`# Produce`) y reglas.
2. `inspect` ejecuta cada tarea cuando sus dependencias están `done`. Una tarea solo queda `done` si el agente
   termina con código 0 **y** produjo lo declarado con frontmatter y evidencias válidas; si no, queda `failed`
   y sus dependientes, bloqueadas. Los reintentos se configuran en `agent.retries` (máximo 10).
   Con Ctrl+C, `inspect` detiene al agente y a sus subprocesos, deja la tarea en curso en `pending`
   y sale con código 130; `adoc resume` la retoma.
3. Cada corrida deja `runs/<fecha>-<id>/run.log` y un log por tarea; en repos git avisa si el agente
   modificó archivos fuera de las rutas permitidas.
4. `sync` vuelve `stale` las notas cuyas fuentes cambiaron (salvo `locked: true`) y reabre sus tareas.

## Tamaño del proyecto

`plan` ajusta cuántas tareas genera según los archivos de **código** (no cuenta docs, datos, imágenes ni
videos; las carpetas ocultas de herramientas como `.devin` o `.vscode` no son módulos):

| Perfil | Archivos de código | Tareas por grupo de módulos | Ejemplo |
|--------|--------------------|-----------------------------|---------|
| `small` | ≤ 150 | 1 (módulos + funcionalidades + tests) | ~45 archivos → 6-7 tareas |
| `medium` | ≤ 1500 | 2 (módulos + funcionalidades, tests) | |
| `large` | > 1500 | 3 (módulos, funcionalidades, tests) | |

Los módulos vecinos se agrupan en una misma tarea hasta `max_files_per_task` / `max_bytes_per_task`.
El perfil es automático (`[plan] size = "auto"`) o fijo (`"small"`, `"medium"`, `"large"`). Si el plan
cambia, `adoc plan --force` lo reemplaza: borra las tareas que sobran y las que no cambian conservan su estado.

## Ver el resultado

```bash
adoc preview            # sitio VitePress con dashboard, abierto en el navegador
adoc report             # reporte HTML único (sin Node), se puede compartir
adoc report --out informe.html --no-open
adoc ask "¿Cómo se calcula el total de una reserva?"   # pregunta al agente → FAQ
```

- **`preview`** prepara el sitio (`site sync`) y lo abre con `vitepress dev --open`. La portada incluye un
  resumen (región gestionada `dashboard` en `docs/index.md`) y el sidebar añade **Estado**, **Hallazgos** y
  **Preguntas y FAQ** y **Grafo del conocimiento**. Sin VitePress, muestra el reporte HTML.
- **`report`** genera un HTML autocontenido (sin CDN ni recursos externos) en el espacio de trabajo, con
  progreso, hallazgos por severidad y evidencias, funcionalidades sin tests, specs, el grafo del
  conocimiento, tareas y preguntas. `--format markdown` incluye el resumen del grafo.
- **Preguntas**: las del agente que bloquean tareas (`task answer`), las dudas para el equipo técnico que los
  agentes registran en `knowledge/memory/open-questions.md`, y la **FAQ**: respuestas de `adoc ask`
  (notas `type: faq` en `knowledge/faq/`) y las dadas con `task answer`.

## Grafo del conocimiento

Las notas del vault y sus relaciones, como el grafo de Obsidian: wikilinks, `related` (hallazgos),
`modules`, tests de cada funcionalidad y el código citado como evidencia (`sources`).

```bash
adoc graph                        # HTML interactivo en el espacio de trabajo, abierto en el navegador
adoc graph --no-open              # solo lo genera
adoc graph --out grafo.html       # HTML en otra ruta, sin abrir el navegador
adoc graph --json                 # JSON (lo usa la extensión; `--format json` sigue funcionando)
```

- **Dónde se ve**: la página HTML de `adoc graph` (`~/.archeology-doc/projects/<proyecto>/graph.html`),
  la sección **Grafo** de `adoc report`, la página **Grafo del conocimiento** del sitio (`docs/grafo.md`,
  enlazada desde la portada y el sidebar; `site sync` la regenera) y la pestaña **Grafo** de la extensión.
  Los cuatro usan el mismo renderizador (`crates/archeology-core/assets/graph.js`, sin dependencias).
- **Interacción**: búsqueda (Enter centra la primera coincidencia), filtro por tipo en la leyenda (índices
  y código ocultos al empezar), zoom con la rueda o `+`/`−`, arrastrar para mover nodos o la vista. Un clic
  abre la nota: en el sitio navega a su página del vault; en el HTML y el reporte muestra título, tipo y
  ruta con enlace `file://`; en la extensión abre el archivo en el editor (el código, en la línea citada).
- **Resumen** (`adoc review`, en texto, markdown y la clave `graph` del JSON): notas, archivos de código,
  relaciones por tipo, las 5 notas más conectadas y las **notas huérfanas** (sin ninguna relación; no
  cuentan los índices/MOCs).
- **Seguridad**: el HTML no carga nada externo y lleva una CSP con nonce; los datos van como JSON escapado
  (un título con `</script>` no rompe la página). El sitio no publica rutas absolutas locales; sus recursos
  van en `docs/public/adoc-graph/` (marcada como generada: si existe sin la marca, `site sync` no la toca).
- Textos: la página de `adoc graph` y el resumen de `review` usan el idioma de la interfaz; el reporte y
  el sitio, el de la documentación del proyecto.

## Uso programático

`adoc context` entrega en una sola llamada lo documentado sobre una zona del código, sin agentes ni red
(solo `knowledge/`, el estado local y `git`). Es el punto de entrada para agentes, scripts y la software factory:

```bash
adoc context src/billing/ --format json            # contrato JSON estable (schema_version 1)
adoc context module:billing feature:checkout       # varios objetivos; texto en la terminal
adoc context src/billing/invoice.py --format markdown --max-chars 4000   # bloque para el prompt de un agente
adoc context --query "redondeo" --include notes,findings
adoc context src/ --fail-on-stale                  # código 3 si alguna nota está desactualizada (CI)
```

Extracto de `adoc context src/billing/ --format json`:

```json
{
  "schema_version": 1,
  "adoc_version": "0.10.22",
  "project": { "id": "4ced…", "root": "/ruta/proj", "knowledge_commit": "2865e3a…", "head_commit": "6b4db88…", "generated_at": "2026-10-31T00:07:34Z" },
  "resolved_targets": [{ "input": "src/billing/", "kind": "path", "matches": ["module:billing", "module:src"] }],
  "unresolved_targets": [],
  "modules": [{ "id": "module:billing", "title": "Facturación", "summary": "Cobros, facturas e impuestos.", "paths": ["src/billing/"], "note": "knowledge/modules/billing.md", "stale": false }],
  "features": [{ "id": "feature:checkout", "title": "Checkout", "summary": "Cobra el carrito y emite la factura.", "modules": ["module:billing"], "note": "knowledge/features/checkout.md", "stale": false }],
  "notes": [{ "path": "knowledge/modules/billing.md", "title": "Facturación", "type": "module", "relevance": 0.9, "stale": false, "stale_reason": null,
              "content": "Cobros, facturas e impuestos.", "truncated": false,
              "evidence": [{ "file": "src/billing/invoice.py", "line": 1, "end_line": 5, "valid": true }] }],
  "findings": [{ "id": "find-rounding", "severity": "high", "category": "contradiction", "kind": "contradiction", "title": "Redondeo distinto",
                 "summary": "El total redondea distinto que el impuesto.", "status": "open",
                 "evidence": [{ "file": "src/billing/invoice.py", "line": 4, "end_line": 99, "valid": false }],
                 "note": "knowledge/findings/rounding.md", "stale": false }],
  "tests": {
    "covered": [{ "feature": "feature:checkout", "scenario": "Cobro exitoso", "tests": ["src/billing/test_invoice.py:3"] }],
    "missing": [{ "feature": "feature:checkout", "scenario": "Tarjeta rechazada" }],
    "disabled_or_empty": [{ "file": "src/billing/test_invoice.py", "line": null, "reason": "disabled" }]
  },
  "dependencies": { "outgoing": [{ "from": "feature:checkout", "to": "module:auth", "depth": 1, "kinds": ["link"] }], "incoming": [] },
  "decisions": [],
  "glossary": [{ "term": "IVA", "definition": "Impuesto al valor agregado.", "note": "knowledge/glossary/iva.md" }],
  "open_questions": [],
  "warnings": ["1 archivo(s) de código cambiaron entre el commit documentado 2865e3a6 y HEAD 6b4db88f: ejecuta `adoc sync` para detectar notas desactualizadas"]
}
```

- **Contrato**: [`docs/context.md`](docs/context.md) y el JSON Schema [`schemas/context.v1.schema.json`](schemas/context.v1.schema.json).
  `--format json` es el formato canónico; `--json` es un alias. `adoc doctor` muestra la versión del contrato
  (`context_schema_version` en su JSON).
- **Criterios compartidos**: `stale` es el de `adoc sync` (marcada, o con una fuente citada que cambió desde el
  último `sync`: `stale_reason` lo explica); `evidence[].valid` es el de `adoc check`. El aviso de commit
  solo aparece si cambió código (no el vault) desde el último `sync`. Nunca se incluyen archivos
  que coinciden con `scan.secrets_patterns`, aunque una nota los cite.

## Cuando el agente pregunta

Los agentes corren sin interfaz, así que nadie puede contestarles en el momento:

1. **Cada tarea les indica no preguntar**: deciden lo más razonable, lo marcan `confidence: low` y anotan la
   duda en `knowledge/memory/open-questions.md`.
2. **Si aun así terminan con una pregunta**, la tarea queda en estado `question` (no se reintenta en vano):
   ```
   ? T-014 pregunta: ¿El descuento máximo correcto es 10 % o 20 %?
     responde con: adoc task answer T-014 "<respuesta>"
   ```
   `task answer` guarda la respuesta en la sección `# Respuestas` de la tarea y la vuelve a `pending`; el
   siguiente `adoc inspect` la ejecuta con esa información. Las tareas que no dependen de ella siguen.
3. **Para tareas complejas**, `adoc task run T-014 --interactive` abre la sesión normal del agente con la
   tarea como primer mensaje; al salir, `adoc` valida la salida y la marca `done`. Disponible para claude,
   codex, devin, gemini, qwen y cursor (`interactive_args` en la config para otros).

## Sitio

VitePress (Node 18+) se busca en este orden:

1. **Proyecto**: `node_modules/vitepress` (`npm i -D vitepress`).
2. **Compartido**: `adoc site setup` lo instala una vez en el directorio de configuración.
3. **Global**: `npm i -g vitepress`.

Con 2 y 3, `site dev|build` enlaza `docs/node_modules` a esa instalación (ignorado en `docs/.gitignore`),
sin tocar el `package.json` del proyecto. En Windows el enlace requiere el modo desarrollador; si no,
usa la opción 1. `adoc doctor` muestra cuál se usará. Para verificar en CI: `adoc check --site --yes`.

## Idioma / Language

`adoc` habla inglés (`en`) y español (`es`). Hay dos idiomas independientes:

**Idioma de la interfaz** (mensajes, errores, avisos). Se elige, de mayor a menor prioridad:

1. el flag global `--lang <en|es>`;
2. la variable de entorno `ADOC_LANG`;
3. `language` en `~/.archeology-doc/config.toml` (o `$ARCHEOLOGY_DOC_CONFIG_DIR/config.toml`);
4. el locale del sistema (`LC_ALL`, `LC_MESSAGES`, `LANG` o el del sistema operativo: `es*` → `es`, `en*` → `en`);
5. `en`.

Un valor inválido se avisa y se pasa a la siguiente fuente.

```bash
adoc config set language es   # guarda la preferencia
adoc config get language      # valor efectivo y su origen (--json para scripts)
adoc config unset language    # vuelve a la detección automática
```

**Idioma de la documentación del proyecto**: el del contenido generado (plantillas, regiones de
`AGENTS.md`/`CLAUDE.md`, notas índice, `llms.txt`, sitio y prompts de las tareas; se le indica al agente
en qué idioma escribir las notas). Se guarda en `archeology-doc.toml`:

```toml
[project]
language = "en"   # en | es
```

`adoc init --lang <en|es>` lo fija; sin `--lang`, se usa el idioma de la interfaz resuelto. Las
configuraciones sin el campo (anteriores a esta versión) se tratan como `es`. La salida para máquinas
(claves JSON, estados como `pending`/`done`/`question`, claves de frontmatter y nombres de archivo) no
cambia con el idioma.

## Variables de entorno

| Variable | Uso |
|----------|-----|
| `ARCHEOLOGY_DOC_CONFIG_DIR` | Directorio de configuración (por defecto `~/.archeology-doc`). Útil en tests y CI. |
| `ADOC_LANG` | Idioma de la interfaz (`en`, `es`); ver [Idioma / Language](#idioma--language). |
| `ARCHEOLOGY_DOC_DOWNLOAD` | Carpeta de descargas para `adoc upgrade` y los instaladores (por defecto `https://archeology-doc.com/download`). |
| `ADOC_NO_UPDATE_CHECK` | `1` desactiva el aviso de versión nueva de `adoc doctor` (sin consultas de red). |
| `RUST_LOG` | Filtro de logs (`-v`/`-vv` equivalen a `info`/`debug`). |

## Extensión de VS Code

`extension/` contiene una interfaz visual sobre el CLI: panel con tareas (ejecutar, interactivo, responder
preguntas), hallazgos con salto a la evidencia, conocimiento por tipo y specs; barra de estado con progreso
y agente; marcas en el editor en las líneas citadas por hallazgos. Ver [extension/README.md](extension/README.md).

Al iniciar comprueba las herramientas: si falta `adoc` ofrece instalarlo (ejecuta el instalador oficial en
un terminal visible y verifica el SHA-256), y si faltan git, Node o VitePress ofrece instalarlos o enlaza su
descarga. Nada se instala sin un clic. Comando: *adoc: Instalar herramientas*; se desactiva con
`archeologyDoc.checkToolsOnStartup`.

```bash
cd extension && npm ci && npm run package
code --install-extension archeology-doc-0.1.0.vsix
```

## Estructura del repo

```
crates/
  archeology-core/      # modelos, estado, plantillas, validación
  archeology-scan/      # scanner e inventario
  archeology-agents/    # orquestador y adaptadores
  archeology-openspec/  # spec bridge
  archeology-registry/  # registro global
  archeology-cli/       # binario
templates/{en,es}/      # plantillas de notas, tareas y sitio, por idioma
crates/archeology-core/assets/           # renderizador del grafo (graph.js/.css), copia única
crates/archeology-core/locales/{en,es}/  # catálogos de mensajes (i18n)
fixtures/               # repos de prueba: small-node, medium-ts, legacy-py, with-openspec
schemas/                # contratos JSON publicados (context.v1.schema.json)
docs/                   # documentación de contratos (context.md)
```

## Desarrollo

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
# build real del sitio (instala VitePress en un directorio temporal; requiere Node y red):
cargo test -- --ignored
```

## Release / Publicar

`scripts/build.sh` es el punto de entrada único para publicar en local: actualiza la
versión, compila binarios, extensión e instaladores (con `WEBSITE/dist.sh`) y arma
`dist/site/`, la carpeta lista para subir a Cloudflare Pages (ignorada por git).

```bash
scripts/build.sh --dry-run --bump patch     # qué cambiaría, sin tocar nada
scripts/build.sh --bump minor --tag         # 0.1.0 → 0.2.0, compila y muestra los comandos git
scripts/build.sh --version 0.2.0 --deploy   # versión fija y despliegue (pide confirmación; --yes para omitirla)
scripts/build.sh --no-bump --targets "x86_64-apple-darwin" --native --skip-vsix   # solo esta máquina
scripts/build.sh --help
```

- **Versión**: `--version X.Y.Z`, `--bump patch|minor|major` o `--no-bump`. Cambia
  `version` en `[workspace.package]` de `Cargo.toml`, refresca `Cargo.lock` y pone la
  misma versión en `extension/package.json` y `package-lock.json`.
- **Compilación**: `--targets`, `--native`, `--skip-docker` y `--skip-vsix` se pasan a
  `WEBSITE/dist.sh`; `--base-url` (por defecto `https://archeology-doc.com/download`) es la
  URL que usan los instaladores; `--clean` vacía `WEBSITE/download` antes. Antes de compilar
  comprueba las herramientas: para Linux/Windows hace falta `cargo-zigbuild` + zig
  (`pip install ziglang cargo-zigbuild`) o Docker en marcha.
- **`dist/site/`**: copia de `WEBSITE/` sin `dist.sh`, `README.md`, `installers/`, `scripts/`
  ni `download/README.md`; falla si quedan marcadores `__ADOC_DOWNLOAD_BASE__` o archivos
  de más de 25 MiB, y verifica `SHA256SUMS`.
- **`--deploy`**: `npx wrangler pages deploy dist/site` en el proyecto
  `$CLOUDFLARE_PAGES_PROJECT` (por defecto `archeology-doc`); requiere `npx wrangler login`
  o `CLOUDFLARE_API_TOKEN` + `CLOUDFLARE_ACCOUNT_ID`.
- **`--tag`**: muestra (no ejecuta) `git commit -am "release vX" && git tag vX && git push origin main vX`.

**En CI**: al subir un tag `v*`, `.github/workflows/release.yml` comprueba que el tag
coincida con la versión de `Cargo.toml`, compila todas las plataformas, crea el release en
GitHub y despliega el sitio en Cloudflare Pages (secrets `CLOUDFLARE_API_TOKEN` y
`CLOUDFLARE_ACCOUNT_ID`; variables opcionales `SITE_URL` y `CLOUDFLARE_PAGES_PROJECT`).

## Licencia

MIT — ver [LICENSE-MIT](LICENSE-MIT).
