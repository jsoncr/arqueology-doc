---
id: test-{{ slug }}
type: test-report
title: "Tests: {{ title }}"
status: draft
confidence: {{ confidence | default("medium") }}
feature: {{ feature }}
covered_scenarios: 0
missing_scenarios: 0
test_files:
{{ test_files }}
sources:
{{ sources }}
generated_by: {{ agent }}
updated: {{ date }}
locked: false
---

# Cobertura de escenarios: {{ title }}

## Escenarios cubiertos

| Escenario | Tests |
|-----------|-------|

## Escenarios faltantes

| Escenario | Tipo | Prioridad sugerida |
|-----------|------|--------------------|

## Tests débiles detectados
