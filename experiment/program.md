# Spack Spec Parser Decomposition Program

## Context

Commit `ea96bb6885884b19d18e4517b9bd76d7eef75d13` reduces parse time from ~1783 μs to ~896 μs.
Your job: apply it in 4 steps, benchmark each, so we know which category of change drives which
portion of the speedup.

**Reference** — the target end state for any file:
```bash
git show ea96bb6885884b19d18e4517b9bd76d7eef75d13 -- <path>
```

## Workflow per step

1. Apply the changes for the current step.
2. Run tests — they MUST pass:
   `source .venv/bin/activate && python3 -m pytest lib/spack/spack/test/spec_syntax.py -x -q 2>/dev/null | tail -3`
3. Run benchmark ONCE:
   `source .venv/bin/activate && experiment/run.sh 2>/dev/null | grep Total`
4. ALWAYS commit (even if no perf improvement — we are documenting):
   `git add -A && git commit --signoff -m "spec_parser.py: <desc>\n\ntotal time: X μs (prev: Y μs)"`
5. Append a row to `experiment/scratchpad.md`.
6. Move to the next step.

---

## Step 3 — Named capture groups + use `_from_version_list_string` (Categories 4, 5)

**What and why:** Add named subgroups to token patterns so the parser extracts data during
matching instead of re-slicing `token.value`. The existing `Tokenizer` infrastructure in
`spack/tokenize.py` already supports this: named groups are renamed with a token-type prefix and
stored in `token.subvalues`. Use this to replace all `token.value` slicing in the parser with
`token.subvalues[...]` access. Crucially, use `_from_version_list_string()` (from step 2) for
`VERSION` tokens — this is the biggest single speedup available on the old architecture since
version parsing was 44% of total parse time.

**Changes to `lib/spack/spack/spec_parser.py` only:**

### 3a. Add subgroups to SpecTokens patterns

```python
# BEFORE:
VERSION_HASH_PAIR = rf"(?:@(?:{GIT_VERSION_PATTERN})=(?:{VERSION}))"
GIT_VERSION = rf"@(?:{GIT_VERSION_PATTERN})"
VERSION = rf"(?:@\s*(?:{VERSION_LIST}))"
PROPAGATED_BOOL_VARIANT = rf"(?:(?:\+\+|~~|--)\s*{NAME})"
BOOL_VARIANT = rf"(?:[~+-]\s*{NAME})"
PROPAGATED_KEY_VALUE_PAIR = rf"(?:{NAME}:?==(?:{VALUE}|{QUOTED_VALUE}))"
KEY_VALUE_PAIR = rf"(?:{NAME}:?=(?:{VALUE}|{QUOTED_VALUE}))"

# AFTER:
VERSION_HASH_PAIR = rf"(?:@(?P<git_version>{GIT_VERSION_PATTERN})=(?P<version_pair>{VERSION}))"
GIT_VERSION = rf"@(?P<git_version>{GIT_VERSION_PATTERN})"
VERSION = rf"(?:@\s*(?P<version_list>{VERSION_LIST}))"
PROPAGATED_BOOL_VARIANT = rf"(?:(?P<bv_prefix>\+\+|~~|--)\s*(?P<bv_name>{NAME}))"
BOOL_VARIANT = rf"(?:(?P<bv_prefix>[~+-])\s*(?P<bv_name>{NAME}))"
PROPAGATED_KEY_VALUE_PAIR = rf"(?:(?P<kv_name>{NAME})(?P<kv_sep>:?==)(?P<kv_value>{VALUE}|{QUOTED_VALUE}))"
KEY_VALUE_PAIR = rf"(?:(?P<kv_name>{NAME})(?P<kv_sep>:?=)(?P<kv_value>{VALUE}|{QUOTED_VALUE}))"
```

### 3b. Update `SpecNodeParser.parse()` to use subvalues

Replace the version/variant/kv parsing block. The `token.subvalues` dict is populated
automatically by `Tokenizer` for tokens with named groups.

**Versions** (the key speedup — replaces `VersionList([from_string(...)])` with fast path):
```python
# BEFORE (one block for all three version tokens):
initial_spec.versions = spack.version.VersionList(
    [spack.version.from_string(self.ctx.current_token.value[1:])]
)
initial_spec.attach_git_version_lookup()

# AFTER (three separate blocks based on which token was accepted):
if self.ctx.accept(SpecTokens.VERSION_HASH_PAIR) or self.ctx.accept(SpecTokens.GIT_VERSION):
    initial_spec.versions = spack.version.VersionList(
        [spack.version.GitVersion(self.ctx.current_token.subvalues["git_version"])]
    )
    initial_spec.attach_git_version_lookup()
elif self.ctx.accept(SpecTokens.VERSION):
    initial_spec.versions = spack.version.VersionList._from_version_list_string(
        self.ctx.current_token.subvalues["version_list"]
    )
```

**Bool variants:**
```python
# BEFORE (BOOL_VARIANT):
name = self.ctx.current_token.value[1:].strip()
variant_value = self.ctx.current_token.value[0] == "+"
# AFTER:
name = self.ctx.current_token.subvalues["bv_name"]
variant_value = self.ctx.current_token.subvalues["bv_prefix"] == "+"

# BEFORE (PROPAGATED_BOOL_VARIANT):
name = self.ctx.current_token.value[2:].strip()
variant_value = self.ctx.current_token.value[0:2] == "++"
# AFTER:
name = self.ctx.current_token.subvalues["bv_name"]
variant_value = self.ctx.current_token.subvalues["bv_prefix"] == "++"
```

**Key-value pairs:**
```python
# BEFORE (KEY_VALUE_PAIR):
name, value = self.ctx.current_token.value.split("=", maxsplit=1)
concrete = name.endswith(":")
if concrete:
    name = name[:-1]
add_flag(name, strip_quotes_and_unescape(value), propagate=False, concrete=concrete)

# AFTER:
name = self.ctx.current_token.subvalues["kv_name"]
sep = self.ctx.current_token.subvalues["kv_sep"]
value = self.ctx.current_token.subvalues["kv_value"]
concrete = sep.startswith(":")
add_flag(name, strip_quotes_and_unescape(value), propagate=False, concrete=concrete)

# BEFORE (PROPAGATED_KEY_VALUE_PAIR):
name, value = self.ctx.current_token.value.split("==", maxsplit=1)
concrete = name.endswith(":")
if concrete:
    name = name[:-1]
add_flag(name, strip_quotes_and_unescape(value), propagate=True, concrete=concrete)

# AFTER:
name = self.ctx.current_token.subvalues["kv_name"]
sep = self.ctx.current_token.subvalues["kv_sep"]
value = self.ctx.current_token.subvalues["kv_value"]
concrete = sep.startswith(":")
add_flag(name, strip_quotes_and_unescape(value), propagate=True, concrete=concrete)
```

Also update `EdgeAttributeParser.parse()` which also uses `KEY_VALUE_PAIR`:
```python
# BEFORE:
name, value = self.ctx.current_token.value.split("=", maxsplit=1)
if name.endswith(":"):
    name = name[:-1]
value = value.strip("'\" ").split(",")
# AFTER:
name = self.ctx.current_token.subvalues["kv_name"]
value = strip_quotes_and_unescape(self.ctx.current_token.subvalues["kv_value"]).split(",")
```

Expected perf impact: **large** — version parsing was 44% of total time; `_from_version_list_string`
replaces the expensive `VersionList` construction path.

---

## Step 4 — Token consolidation (Category 4)

**What and why:** Merge the three version token types into one, the two bool-variant types into
one, the two KVP types into one, and `DEPENDENCY` + `START_EDGE_PROPERTIES` into one. Each merged
token uses the subgroups from step 3 to distinguish cases. Fewer token types = fewer branches in
the parser loop and fewer enum comparisons.

**Changes to `lib/spack/spack/spec_parser.py` only:**

### 4a. Remove separate token types from SpecTokens, replace with consolidated ones

```python
# REMOVE these from SpecTokens:
#   START_EDGE_PROPERTIES
#   VERSION_HASH_PAIR
#   GIT_VERSION
#   PROPAGATED_BOOL_VARIANT
#   PROPAGATED_KEY_VALUE_PAIR

# REPLACE/UPDATE remaining tokens to cover both cases:

# DEPENDENCY now covers START_EDGE_PROPERTIES too:
DEPENDENCY = rf"(?:(?:\^|\%\%|\%)(?:(?P<edge_bracket>\[)|(?:\s*{VIRTUAL_ASSIGNMENT})?))"

# VERSION now covers all three version token types:
VERSION = (
    rf"@(?:(?P<git_version>{GIT_VERSION_PATTERN}(?:={VERSION})?)"
    rf"|\s*(?P<version_list>{VERSION_LIST}))"
)

# BOOL_VARIANT now covers propagated variants too (bv_prefix is 1 or 2 chars):
BOOL_VARIANT = rf"(?P<bv_prefix>\+\+|~~|--|[~+-])\s*(?P<bv_name>{NAME})"

# KEY_VALUE_PAIR now covers propagated KVP too (kv_sep is :?==? — matches = or == or := or :==):
KEY_VALUE_PAIR = rf"(?P<kv_name>{NAME})(?P<kv_sep>:?==?)(?P<kv_value>{VALUE}|{QUOTED_VALUE})"
```

### 4b. Update parser dispatch to use single branches

In `next_spec()`, replace:
```python
if self.ctx.accept(SpecTokens.START_EDGE_PROPERTIES):
    has_edge_attrs = True
elif self.ctx.accept(SpecTokens.DEPENDENCY):
    has_edge_attrs = False
```
with:
```python
if not self.ctx.accept(SpecTokens.DEPENDENCY):
    break
has_edge_attrs = bool(self.ctx.current_token.subvalues and
                      self.ctx.current_token.subvalues.get("edge_bracket"))
```

In `SpecNodeParser.parse()`, replace the three-way version accept:
```python
if (self.ctx.accept(SpecTokens.VERSION_HASH_PAIR)
        or self.ctx.accept(SpecTokens.GIT_VERSION)
        or self.ctx.accept(SpecTokens.VERSION)):
    if self.ctx.current_token.subvalues and self.ctx.current_token.subvalues.get("git_version"):
        initial_spec.versions = spack.version.VersionList(
            [spack.version.GitVersion(self.ctx.current_token.subvalues["git_version"])]
        )
        initial_spec.attach_git_version_lookup()
    else:
        initial_spec.versions = spack.version.VersionList._from_version_list_string(
            self.ctx.current_token.subvalues["version_list"]
        )
```

Replace the two bool-variant branches with one:
```python
elif self.ctx.accept(SpecTokens.BOOL_VARIANT):
    prefix = self.ctx.current_token.subvalues["bv_prefix"]
    name = self.ctx.current_token.subvalues["bv_name"]
    propagate = len(prefix) == 2
    variant_value = prefix[0] == "+"
    add_flag(name, variant_value, propagate=propagate, concrete=True)
```

Replace the two KVP branches with one:
```python
elif self.ctx.accept(SpecTokens.KEY_VALUE_PAIR):
    name = self.ctx.current_token.subvalues["kv_name"]
    sep = self.ctx.current_token.subvalues["kv_sep"]
    value = self.ctx.current_token.subvalues["kv_value"]
    propagate = "==" in sep
    concrete = sep.startswith(":")
    add_flag(name, strip_quotes_and_unescape(value), propagate=propagate, concrete=concrete)
```

Expected perf impact: **small-moderate** — fewer token types and branches.

---

## Step 5 — Full architectural shift: scanner + no Token/TokenContext + flat parser (Categories 1, 2, 3)

**What and why:** Replace the `Tokenizer` + `TokenContext` + `Token` objects with a single
compiled `FAST_SPEC_REGEX` and a direct scanner. This eliminates:
- **Category 1 (fewer allocations):** no `Token` object per match — use `re.Match` directly
- **Category 2 (reduced fn calls):** no `SpecNodeParser` / `EdgeAttributeParser` / `FileParser`
  classes; no `accept()` / `expect()` on TokenContext
- **Category 3 (reduced iterations):** `\s*` prefix in `FAST_SPEC_REGEX` means whitespace is
  consumed at C level, never producing a Python object

By now (after steps 3–4), the token types are already consolidated and use subgroups — so this
step is purely an architectural swap with no logic changes needed.

Apply the full diff for these files from `ea96bb688...`:
- `lib/spack/spack/spec_parser.py`
- `lib/spack/docs/conf.py`
- `lib/spack/spack/cmd/style.py`
- `lib/spack/spack/test/spec_syntax.py`

The key structural changes (verify your implementation matches):
- `SpecTokens` becomes a plain string namespace (not `TokenBase` enum)
- `RAW_PATTERNS` list + group name mangling builds `FAST_SPEC_REGEX`
- `SpecParser.__init__`: `scanner = FAST_SPEC_REGEX.scanner(text)`, `curr = scanner.match()`, `next = scanner.match()`
- All dispatch: `curr.lastgroup == "BOOL_VARIANT"` instead of `ctx.accept(SpecTokens.BOOL_VARIANT)`
- All subgroup access: `curr.group("BOOL_VARIANT_bv_prefix")` (note: mangled names) instead of `token.subvalues["bv_prefix"]`
- No `from spack.tokenize import ...`
- No `Token`, `TokenContext`, `SpecNodeParser`, `EdgeAttributeParser`, `FileParser`

Expected perf impact: **large** — eliminates per-match Python object allocation and removes
the class-call overhead from `accept()` / `SpecNodeParser.parse()` etc.

---

## Notes

- Do NOT try to split step 5 further — the Token-object removal and class flattening are
  deeply interleaved in the diff and cannot produce a passing intermediate state alone.
- Steps 3 and 4 each leave the parser fully functional on the old `Tokenizer` architecture.
- The benchmark runs 10,000 iterations per spec; run it ONCE per step.
- Do NOT add module-level caches keyed on input strings.
