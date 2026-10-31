## Contexto para agentes (archeology-doc)

Este proyecto está documentado con **archeology-doc**. El conocimiento vive en
`{{ knowledge_dir }}/` (vault Markdown compatible con Obsidian).

### Al iniciar una sesión

1. Lee `{{ knowledge_dir }}/00-overview.md`.
2. Lee la memoria estable: `{{ knowledge_dir }}/memory/facts.md`, `conventions.md`,
   `decisions.md` y `open-questions.md`.
3. Revisa los hallazgos abiertos en `{{ knowledge_dir }}/findings/` (`state: open`).
4. Antes de cambiar una zona del código, ejecuta `adoc context <ruta> --format markdown`
   (o `--format json`): sus notas, hallazgos abiertos, tests y dependencias en una llamada.

### Al terminar una sesión

- Crea `{{ knowledge_dir }}/memory/sessions/<AAAA-MM-DD>-<tema>.md` (memoria
  temporal, plantilla `_templates/session.md`) con lo aprendido.
- Solo lo **verificado en el código** pasa a la memoria estable, como entrada
  `- <AAAA-MM-DD> · <afirmación> · fuente: <ruta:línea>`.

### Reglas

- Cita cada afirmación con `ruta:línea`; si no puedes verificarla, márcala
  `confidence: low`.
- Las reglas de negocio documentadas están en `{{ knowledge_dir }}/features/` y
  `{{ knowledge_dir }}/modules/`.
- Nunca modifiques `openspec/specs/` directamente; los cambios van en
  `openspec/changes/`.
- No edites notas con `locked: true` ni el contenido de las regiones
  `<!-- archeology-doc:begin ... -->`: solo archeology-doc las regenera.
- Escribe las notas en español.
