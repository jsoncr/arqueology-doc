## Agent context (archeology-doc)

This project is documented with **archeology-doc**. The knowledge lives in
`{{ knowledge_dir }}/` (Obsidian-compatible Markdown vault).

### When starting a session

1. Read `{{ knowledge_dir }}/00-overview.md`.
2. Read the stable memory: `{{ knowledge_dir }}/memory/facts.md`, `conventions.md`,
   `decisions.md` and `open-questions.md`.
3. Review the open findings in `{{ knowledge_dir }}/findings/` (`state: open`).
4. Before changing a zone of the code, run `adoc context <path> --format markdown`
   (or `--format json`): its notes, open findings, tests and dependencies in one call.

### When ending a session

- Create `{{ knowledge_dir }}/memory/sessions/<YYYY-MM-DD>-<topic>.md` (temporary
  memory, template `_templates/session.md`) with what you learned.
- Only what is **verified in the code** goes into the stable memory, as an entry
  `- <YYYY-MM-DD> · <statement> · source: <path:line>`.

### Rules

- Cite every statement with `path:line`; if you cannot verify it, mark it
  `confidence: low`.
- The documented business rules are in `{{ knowledge_dir }}/features/` and
  `{{ knowledge_dir }}/modules/`.
- Never modify `openspec/specs/` directly; changes go in
  `openspec/changes/`.
- Do not edit notes with `locked: true` or the content of the
  `<!-- archeology-doc:begin ... -->` regions: only archeology-doc regenerates them.
- Write the notes in English.
