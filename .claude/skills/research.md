# Research Document Skill

Conduct structured technical research and produce a comprehensive document in `docs/research/`.

## Usage

`/research {topic}`

## Workflow

### Phase 1: Investigation

Launch 2-3 Explore agents in parallel to gather information:
- **Agent 1:** Research the topic externally (web search for crates, papers, prior art, benchmarks)
- **Agent 2:** Explore the codebase for relevant architecture, existing patterns, and integration points
- **Agent 3 (optional):** Research how other tools/projects solve the same problem

### Phase 2: Write Document

Create `docs/research/{NN}-{slug}.md` where `{NN}` is the next sequential number.

Follow this structure:
1. **Executive Summary** — Problem, recommendation, key trade-offs (1 paragraph)
2. **Current Architecture & Gap Analysis** — What exists today, with file:line references
3. **Requirements** — What we need, derived from VISION.md and current architecture
4. **Options Analysis** — Each option with: description, strengths, weaknesses, Rust crate/version
5. **Comparison Matrix** — Table comparing all options across key criteria
6. **VISION.md Alignment** — How recommendations align with or deviate from the vision
7. **Recommended Architecture** — Phased approach (Phase 1 = simple, Phase 2 = scale, Phase 3 = future)
8. **Integration Design** — Traits, types, data flow diagrams showing how it fits the codebase
9. **Red/Green Team** — (placeholder, filled after Phase 3 audit)
10. **Sources** — URLs, papers, crate links, internal doc references with file:line

Target: 400-500 lines. Use tables for comparisons. Include code snippets for trait definitions and key types.

### Phase 3: Red/Green Team Audit

Dispatch 3 agents in parallel:

1. **Green Team Agent:** Validate all factual claims — check crate versions exist, verify API claims, confirm benchmark numbers, validate math calculations, check that cited features actually work as described.

2. **Red Team Agent:** Challenge every recommendation — argue for alternatives, identify missing options, find scenarios where the recommendation fails, challenge arbitrary thresholds or numbers, check for familiarity bias, identify risks not discussed.

3. **Code Accuracy Agent:** Verify all file:line references match actual code. Check that types, methods, and enums listed match reality. Confirm architectural claims about the codebase.

### Phase 4: Fix and Commit

1. Incorporate all audit findings into the document (update Red/Green Team section, fix errors, address gaps)
2. Commit: `docs: add research on {topic}`

## Style Guide

- Use `>` blockquote for the date/summary line at the top
- Use `---` horizontal rules between major sections
- Use tables for comparisons (not prose lists)
- Include code blocks for trait definitions, SQL schemas, type definitions
- File:line references use backtick format: `` `file.rs:123-456` ``
- Phase recommendations: Phase 1 = simplest viable, Phase 2 = production scale, Phase 3 = future/remote
- Be honest in Red/Green Team — real weaknesses, not strawmen
