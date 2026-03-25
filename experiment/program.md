# Spack Import Optimization Program

## Context

Spack is a Python package manager. Its startup time is dominated by Python import overhead.
The goal is to reduce the wall-clock time of `spack list` (and similar lightweight commands)
by deferring or eliminating expensive imports that are not needed for those commands.

The benchmark is: `/usr/bin/time -f '%e' /usr/bin/python3 ~/spack/bin/spack list > /dev/null`

Baseline: **~0.37s** (very stable, <1% variance across runs).

## Files

**You may modify** (Spack source):
- Any file under `lib/spack/spack/` — move top-level imports into functions, use lazy import
  wrappers, or guard heavy imports behind `if TYPE_CHECKING:` where applicable.

**Do not modify:**
- `experiment/run.sh` — the benchmark script (fixed)

**Scratchpad** (always update by appending):
- `experiment/scratchpad.md` — append-only log table. Each new row goes at the end. Never edit
  earlier rows.
- `experiment/next_ideas.md` — ideas to try next. Edit freely: add new ideas, strike through
  completed ones (`~~item~~`).

Log table format (rows appended to end of `scratchpad.md`):

| # | Change | Time (s) | vs best | Notes |
|---|--------|----------|---------|-------|
| 1 | baseline | 0.37 | — | spack list, no warmup |

**Commit message format:**

```
experiment: <short description>

time: X.XXs (prev: Y.YYs)
```

## Import cost map (as of baseline)

Captured with `/usr/bin/python3 -X importtime bin/spack list 2>&1 | sort -t'|' -k2 -rn | head -40`.
Numbers are cumulative microseconds (flaky due to GC, but order-of-magnitude is reliable).

| cumulative (µs) | self (µs) | module |
|-----------------|-----------|--------|
| 155651 | 478 | spack.main |
| 101490 | 389 | spack.cmd |
| 78882 | 288 | spack.concretize (via spack.cmd) |
| 78515 | 282 | spack.compilers.config (via concretize) |
| 42841 | 1162 | spack.config |
| 34291 | 117 | spack.detection (via compilers.config) |
| 32477 | 426 | spack.detection.common |
| 29712 | 4041 | spack.spec |
| 27952 | 185 | spack.vendor.jsonschema |
| 21015 | 152 | spack.environment |
| 20863 | 2128 | spack.environment.environment |
| 19286 | 215 | spack.hash_types (via repo) |
| 19072 | 1634 | spack.repo |
| 19027 | 418 | spack.paths |
| 15118 | 907 | spack.fetch_strategy |
| 13103 | 1402 | spack.llnl.util.filesystem |
| 11476 | 1508 | spack.oci.opener |
| 10717 | 133 | spack.util.remote_file_cache |
| 10422 | 2071 | spack.util.path |
| 8520 | 352 | spack.util.git |
| 8077 | 2442 | spack.solver.asp |
| 7945 | 521 | spack.util.spack_yaml |
| 7425 | 159 | spack.vendor.ruamel.yaml |
| 7345 | 472 | spack.util.executable |

## Key diagnosis

The root of the bloat is `lib/spack/spack/cmd/__init__.py`, which has these **top-level imports**
even though `spack list` needs none of them:

```python
import spack.concretize       # pulls in compilers.config, detection, … → ~78ms
import spack.environment as ev # → ~21ms
import spack.store             # → pulls spack.database, etc.
import spack.spec_parser       # → ~3ms self
import spack.spec              # → ~30ms self
import spack.repo              # → ~19ms
import spack.util.spack_yaml as syaml
import spack.user_environment as uenv
```

`lib/spack/spack/main.py` adds:
```python
import spack.solver.asp        # → ~8ms
import spack.environment       # (again)
import spack.store             # (again)
```

`spack list` only needs the package repo to enumerate package names. Everything else
(concretizer, environment, compiler detection, OCI, fetch strategies) is dead weight for this
command.

## Approach

The standard Python technique is to move imports from module scope into the body of the
functions that actually need them. For frequently-called helper functions in `spack.cmd` that
are used by *all* commands, a lazy import wrapper is preferable:

```python
# Instead of:
import spack.concretize

# At module level, replace with nothing (or TYPE_CHECKING guard).
# Inside functions that need it:
def foo():
    import spack.concretize
    spack.concretize.do_thing()
```

Python caches modules in `sys.modules`, so the second call to `import spack.concretize` is
essentially free (dict lookup). The cost is paid only once, on first use.

For `TYPE_CHECKING`-only imports (used only in type annotations), use:
```python
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    import spack.concretize
```
and use string annotations (`"spack.concretize.Concretizer"`) in function signatures.

## Strategy

Work through the import cost map from top to bottom. For each expensive import in `spack.cmd`
or `spack.main`:

1. Check which functions in that file actually use it.
2. If only a subset of functions use it, move the import into those functions.
3. Run the benchmark after each change.
4. If the import is only used in type annotations, move it under `TYPE_CHECKING`.

**Measurement note:** `importtime` numbers are noisy (GC interference). Use
`/usr/bin/time -f '%e'` wall-clock measurements (run 3× and take the median) as the ground
truth. The two numbers should roughly agree directionally.

## How experiments work

1. Read this file, `experiment/scratchpad.md`, `experiment/next_ideas.md`, and the relevant
   source files before making any changes.
2. Pick **one focused change** (one import moved, or one module cleaned up).
3. Apply the change.
4. Run `experiment/run.sh` — it outputs wall-clock time.
5. Compare to previous best.
6. **If improved** → commit with the structured message format above.
7. **If not improved** → `git checkout -- <changed files>` to revert.
8. **Always** append a row to `experiment/scratchpad.md` and update `experiment/next_ideas.md`.
9. Repeat.
