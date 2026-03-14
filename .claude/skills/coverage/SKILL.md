---
name: coverage
description: Run code coverage analysis with cargo-llvm-cov. Reports summary stats, flags low-coverage modules, and generates an HTML report.
---

# Code Coverage Skill

Run code coverage analysis and report results.

## Usage

`/coverage`

## Workflow

### Step 1: Run coverage

Run `cargo llvm-cov --html` in the project root. This instruments the test binary with LLVM's source-based coverage and produces an HTML report.

### Step 2: Parse and report

After the coverage run completes, also run `cargo llvm-cov --text` to get a per-file summary table. Report to the user:

1. **Overall line coverage %**
2. **Top 5 lowest-coverage source files** (excluding test files and generated code)
3. **Any source files with 0% coverage** (likely dead code or untested modules)

### Step 3: Suggest improvements

Based on the results, suggest which files or modules would benefit most from additional tests. Prioritize:
- Core logic files (graph/, llm/, tool_executor/) over UI files (tui/)
- Files with many uncovered branches over files with high line coverage but low branch coverage
- Recently changed files (check `git log --oneline -10`)
