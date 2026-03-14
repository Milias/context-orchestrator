---
name: coverage
description: Run code coverage analysis with cargo-llvm-cov. Produces JSON per-file stats, an HTML report, and actionable improvement suggestions.
---

# Code Coverage Skill

Run code coverage analysis and report per-file results.

## Usage

`/coverage`

## Workflow

### Step 1: Run tests with coverage and generate HTML report

```bash
cargo llvm-cov --html
```

This instruments the test binary, runs all tests, collects profraw data, and generates an HTML report at `target/llvm-cov/html/index.html`.

If this step fails with compilation errors, stop and tell the user: "Coverage requires a clean build. Please fix the compilation errors above and re-run `/coverage`."

If tests fail but compilation succeeds, note the failures but continue to report generation (partial coverage data is still useful).

### Step 2: Generate JSON summary and parse results

Run this command to extract per-file coverage data from the same profraw data (does NOT re-run tests):

```bash
cargo llvm-cov report --json --summary-only 2>/dev/null | python3 -c "
import sys, json

data = json.load(sys.stdin)
d = data['data'][0]
totals = d['totals']['lines']
files = d.get('files', [])

print(f'## Overall Coverage')
print(f'Lines: {totals[\"covered\"]}/{totals[\"count\"]} ({totals[\"percent\"]:.1f}%)')
print()

src_files = []
for f in files:
    name = f['filename']
    if '_tests.rs' in name or '/tests.rs' in name or 'tests/' in name:
        continue
    s = f['summary']['lines']
    if '/src/' in name:
        name = 'src/' + name.split('/src/', 1)[1]
    src_files.append((name, s['percent'], s['covered'], s['count']))

src_files.sort(key=lambda x: x[1])

zero = [f for f in src_files if f[1] == 0.0 and f[3] > 0]
if zero:
    print(f'## Files at 0% Coverage ({len(zero)} files)')
    for name, pct, cov, total in zero:
        print(f'  {name} ({total} lines)')
    print()

low = [f for f in src_files if 0.0 < f[1] < 50.0][:10]
if low:
    print(f'## Lowest Coverage Files (non-zero)')
    for name, pct, cov, total in low:
        print(f'  {pct:5.1f}% ({cov}/{total}) {name}')
    print()

high = sorted(src_files, key=lambda x: -x[1])[:5]
if high:
    print(f'## Highest Coverage Files')
    for name, pct, cov, total in high:
        print(f'  {pct:5.1f}% ({cov}/{total}) {name}')
"
```

### Step 3: Present results and suggest improvements

Report the output from Step 2 verbatim. Then add:

1. The HTML report path: `target/llvm-cov/html/index.html`
2. Suggest which files most need tests, prioritizing:
   - Core logic files (graph/, app/, tool_executor/) over UI files (tui/)
   - 0% files with >20 lines (likely missing test coverage, not stubs)
   - Low-coverage files that changed recently (check `git log --oneline -10 -- <file>`)
