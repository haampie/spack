# Spack Spec Parser Decomposition Program

## Context

Commit `ea96bb6885884b19d18e4517b9bd76d7eef75d13` ("spec_parser.py: rewrite for speed") is a
squashed rewrite that reduces parse time from ~1783 μs to ~896 μs (~2×). Your job is to decompose
it into logical steps, benchmark each one, and commit them individually so we understand exactly
which category of change drives how much of the speedup.

**Reference commit** (the target end state):
```bash
git show ea96bb6885884b19d18e4517b9bd76d7eef75d13 -- <file>
```
Use this to see the exact diff for any file.

## Workflow per step

1. Apply the changes for the current step (described below).
2. Run tests — they MUST pass: `source .venv/bin/activate && python3 -m pytest lib/spack/spack/test/spec_syntax.py -x -q 2>/dev/null | tail -3`
3. Run benchmark ONCE: `source .venv/bin/activate && experiment/run.sh 2>/dev/null | grep Total`
4. ALWAYS commit, even if no perf improvement (we are documenting, not gating on speedup): `git add -A && git commit --signoff`
5. Append a row to `experiment/scratchpad.md`.
6. Move to the next step.

### Commit message format

```
spec_parser.py: <category> — <one-line description>

total time: X.XX μs (prev: Y.YY μs)
```

### Scratchpad row format

| # | Step | Category | Total (μs) | Δ vs prev | Notes |
|---|------|----------|------------|-----------|-------|

## Ordered steps

### Step 1 — `spec.py` micro-optimizations (independent of parser)

Apply all five changes from `git show ea96bb688... -- lib/spack/spack/spec.py`:

1. `_valid_compiler_flags`: change list `[...]` → tuple `(...)`
2. `SpecAnnotations`: add `__slots__ = ("original_spec_format", "compiler_node_attribute")`
3. `Spec.__init__`: replace `for h in ht.HASHES: setattr(self, h.attr, None)` with four explicit
   assignments: `self._hash = None`, `self._package_hash = None`, `self._full_hash = None`,
   `self._build_hash = None`
4. `Spec.__init__`: replace `self.external_modules = Spec._format_module_list(external_modules)`
   with `if external_modules: self.external_modules = list(external_modules) else: self.external_modules = None`;
   remove the `_format_module_list` static method
5. `_add_flag`: remove `valid_flags = FlagMap.valid_compiler_flags()` local variable; use
   `_valid_compiler_flags` directly in the `elif name in ...` check

Files: `lib/spack/spack/spec.py` only.

---

### Step 2 — `version_types.py`: add `_from_version_list_string()` (prereq, no speedup yet)

Apply `git show ea96bb688... -- lib/spack/spack/version/version_types.py`.
Adds a 24-line static method after `from_dict()`. No callers yet — no perf change expected.

Files: `lib/spack/spack/version/version_types.py` only.

---

### Step 3 — **Sub-string parsing**: named capture groups (Category 5)

*Avoids double-parsing token values and eliminates the per-token subgroup dict allocation that the
old code created when named groups were used.*

The old `SpecTokens` patterns have no subgroups — the parser slices `token.value` manually.
Add named subgroups to the following patterns in `SpecTokens` and update the `Token` class (from
`spack.tokenize`) to expose `.group(name)` forwarding to the underlying match object:

| Token | Subgroups to add |
|-------|-----------------|
| `BOOL_VARIANT` | `(?P<bv_prefix>[~+-])`, `(?P<bv_name>{NAME})` |
| `PROPAGATED_BOOL_VARIANT` | `(?P<bv_prefix>\+\+\|~~\|--)`, `(?P<bv_name>{NAME})` |
| `KEY_VALUE_PAIR` | `(?P<kv_name>{NAME})`, `(?P<kv_sep>:?=)`, `(?P<kv_value>...)` |
| `PROPAGATED_KEY_VALUE_PAIR` | same with `kv_sep` matching `:?==` |
| `VERSION` | `(?P<version_list>{VERSION_LIST})` |
| `GIT_VERSION` / `VERSION_HASH_PAIR` | `(?P<git_version>...)` |

Update the parser (SpecNodeParser / SpecParser) to use `token.group("bv_name")` etc. instead of
slicing `token.value`.

Reference: subgroup names in the new `SpecTokens` string constants in ea96bb688..., and usage in
`_parse_node`.

Files: `lib/spack/spack/spec_parser.py` (and possibly `lib/spack/spack/tokenize.py`).

---

### Step 4 — **Reduced branching**: consolidate paired token types (Category 4)

*Fewer token types → fewer enum comparisons and fewer branches in the parser loop.*

Using the subgroups from Step 3, merge paired tokens into single tokens:

| Before | After | Distinguishing subgroup |
|--------|-------|------------------------|
| `BOOL_VARIANT` + `PROPAGATED_BOOL_VARIANT` | `BOOL_VARIANT` | `bv_prefix` length (1 vs 2 chars) |
| `KEY_VALUE_PAIR` + `PROPAGATED_KEY_VALUE_PAIR` | `KEY_VALUE_PAIR` | `kv_sep` contains `==` |
| `VERSION` + `GIT_VERSION` + `VERSION_HASH_PAIR` | `VERSION` | `git_version` subgroup present |
| `DEPENDENCY` + `START_EDGE_PROPERTIES` | `DEPENDENCY` | `edge_bracket` subgroup present |

Update all parser call sites to check the subgroup instead of the token type.

Reference: new `SpecTokens` string constants and the `next_spec()` / `_parse_node()` dispatch in
ea96bb688...

Files: `lib/spack/spack/spec_parser.py` only.

---

### Step 5 — **Reduced iterations**: remove WS token (Category 3)

*Whitespace is consumed implicitly by the regex engine at C level instead of producing Python
Token objects that the parser loop must skip.*

- Add `\s*` prefix to each pattern in `SpecTokens` (so whitespace is absorbed before each token)
  OR pass it as a prefix to `Tokenizer` compilation
- Remove `WS = r"(?:\s+)"` from `SpecTokens`
- Remove (or simplify) `parseable_tokens()` — no longer needs to filter WS

Reference: the `\s*(?:...)` structure of `FAST_SPEC_REGEX` and the absence of a WS pattern in
ea96bb688...

Files: `lib/spack/spack/spec_parser.py` only.

---

### Step 6 — **Fewer allocations + reduced function calls**: architectural shift (Categories 1+2)

*Replace Token wrapper objects with direct re.Match usage; eliminate TokenContext, SpecNodeParser,
EdgeAttributeParser, FileParser; inline accept/expect; use `_from_version_list_string` (Step 2).*

By now patterns are consolidated with subgroups and WS is gone. The shift:

- Build `FAST_SPEC_REGEX` from `RAW_PATTERNS` (alternated patterns with group name mangling)
- `SpecParser.__init__`: `scanner = FAST_SPEC_REGEX.scanner(text)`; `self.curr = scanner.match()`;
  `self.next = scanner.match()`
- Replace `TokenContext.advance()` with `curr, next = next, scanner.match()`
- Inline `SpecNodeParser._parse_node()` → `SpecParser._parse_node()`
- Inline `EdgeAttributeParser` → bracket block in `next_spec()`
- Inline `FileParser` → FILENAME branch in `_parse_node()`
- Remove `TokenContext`, `accept()`, `expect()` — dispatch on `curr.lastgroup == "BOOL_VARIANT"`
  etc.
- Call `VersionList._from_version_list_string()` for non-git versions (first real benefit of Step 2)

This step must also land the three adapter files that reference the old `SpecTokens` API:
- `lib/spack/docs/conf.py`
- `lib/spack/spack/cmd/style.py`
- `lib/spack/spack/test/spec_syntax.py`

Reference: the full new `spec_parser.py` and adapter diffs in ea96bb688...

Files: all of the above.

---

## Notes

- Do NOT try to split Step 6 further — the Token-object removal and class flattening are
  deeply interleaved in the diff and cannot produce a passing intermediate state alone.
- Steps 3–5 each leave the parser fully functional on the old `Tokenizer` architecture.
- The benchmark runs 10,000 iterations per spec; one run is sufficient per step.
- Do NOT add module-level caches keyed on input strings.
