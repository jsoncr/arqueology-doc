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

# Scenario coverage: {{ title }}

## Covered scenarios

| Scenario | Tests |
|----------|-------|

## Missing scenarios

| Scenario | Type | Suggested priority |
|----------|------|--------------------|

## Weak tests detected
