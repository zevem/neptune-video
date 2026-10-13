# Domain Docs

## Layout

Single-context: `GLOSSARY.md` at the repo root and ADRs in `docs/adr/`.

## Before exploring

Read the glossary and any ADRs relevant to the area being explored.

If these files are absent, proceed silently. The `domain-modeling` skill
creates them lazily when terms or decisions are resolved.

## Vocabulary

Use glossary terms when naming domain concepts in issues, proposals,
hypotheses, and tests. Follow any explicit guidance about synonyms.

If a needed concept is missing, reconsider whether it belongs to the
domain or note the gap for `domain-modeling`.

## ADR conflicts

Explicitly identify proposals that contradict an existing ADR, naming
the ADR and explaining why the decision should be reconsidered.
