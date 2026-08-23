# Curiosities of the Python spec implementation

Oddities of the Python implementation discovered while porting the spec model to Rust
(branch `hs/experiment/rust-spec`). The Rust port reproduces all of these bug-for-bug
unless marked otherwise. Append new findings per area; cite `file:line` against the
branch state at the time of writing.

## Versions (`spack/version/version_types.py`)

- `1.0final` parses equal to `1.0`: the `final` prerelease tag is trimmed and the
  release tuple compares equal.
- `@=1.2=3` is not a pinned standard version: `=1.2=3` lexes as a *git ref* `1.2`
  pinned at version `3` (the git-version `=` split runs before standard parsing).
- Range endpoints stringify lazily through `_prev_version`/`_next_version`, so a
  `ClosedOpenRange` prints bounds that were never materialized as parsed versions;
  `Version("10.0001")` restringifies as `10.1` after component round-tripping.
- Error taxonomy is inconsistent at the edges: `StandardVersion.union(GitVersion)`
  and `GitVersion.__contains__` raise bare `NotImplementedError`;
  `VersionList.intersects(GitVersion)` and `ClosedOpenRange.intersection(VersionList)`
  raise `TypeError`.
- `GitRefLookup.get()` returns a **list** (not the documented tuple) when its result
  was loaded from the JSON cache file — callers relying on tuple-ness fail
  intermittently depending on cache state.

## Targets (`spack/spec.py` target helpers, archspec)

- `_target_intersection` output **order is hash-seed dependent**: it iterates a `set`
  of `Microarchitecture` objects, so `:cannonlake ∩ :cascadelake` returns
  `[':skylake', ':x86_64_v4']` or the reverse depending on the process. Only the
  canonical (sorted) form is deterministic. (Rust uses table order; canonical strings
  are byte-identical.)
- `:unknown_target` canonicalizes to plain `unknown_target`: a generic target is its
  own family, so the "upper bound is a root → singleton" rule fires, e.g.
  `:zen9` → `zen9`.
- `ArchSpec.__init__` hits `UnboundLocalError` for an argument that is not an
  ArchSpec/str/tuple, and an unpack `ValueError` for a wrong-length tuple.
- The reserved-name `os`/`target` setters assign the platform *before* raising the
  "isn't the current platform" `ValueError`, leaving the ArchSpec half-updated.

## Variants (`spack/variant.py`)

- `str(Spec("foo=*"))` renders `foo='*'`: `quote_if_needed` quotes the `*` because it
  is not in the no-quotes character class, so the round-trip goes through the quoted
  form.
- `VariantValue.set` is `tuple(sorted(set(values)))`: dedupe happens *before* the
  sort, so duplicate mixed-type values that dedupe to one survive, while two or more
  distinct mixed `str`/`bool` values raise `TypeError` from `sorted`.

## Tokenizer / parser (`spack/spec_parser.py`, `spack/tokenize.py`)

- `VERSION`'s trailing `\b` backtracks: `@:2.7=t` lexes the upper bound as `2.`
  (not `2`) because the word boundary holds on the shorter prefix.
- `VERSION_RANGE`'s `(?!\s*=)` lookahead only bites at the end of the run:
  `@1.2:develop = foo` lexes as `@1.2:` plus a package name.
- `foo==§` tokenizes as a KEY_VALUE_PAIR `foo==` with value `=` — `=` is itself a
  VALUE character, so the propagated-pair failure falls back into a plain pair.
- A quoted value with a trailing escaped quote at EOF still closes: `"a\"` matches,
  because backtracking hands the last escaped quote to the closer.
- `FILENAME` is greedy to the *last* `.json`/`.yaml` in the run: `./a.json.yaml`
  is one filename, `./a.jsonx` lexes as `./a.json` + `x`.
- `@` + 41 hex chars: ordered alternation beats longest match — `GIT_VERSION`
  consumes exactly 40, and the 41st character starts a package name.
- A stray `]` makes `SpecParser.all_specs()` **loop forever** (no token consumed, no
  error). The Rust parser returns a no-progress error instead (deliberate deviation).
- `UNEXPECTED` consumes one character plus trailing whitespace (`.[\s]*`), which the
  caret/underline error messages depend on.
- Tokenization is lazy relative to parsing: `x@1.2@2.3 =` reports a tokenization
  error but `x@1.2@2.3 y =` reports the duplicate-version parse error first, because
  the parser pulls tokens with one-token lookahead.
- The same laziness makes edge attachment order observable. `next_spec` attaches a
  dependency the moment its node is parsed, before the loop pulls another token, so
  `x %[virtuals=c]gcc %[virtuals=c]clang %&` reports the `_add_dependency` conflict on
  `clang` and never reaches the tokenization error on `&`. An event-replaying parser
  has to mark that exact point (`SpecAction::EndDependencyNode`) to agree.
- `_parse_node` refuses a dependent of a concrete root *before* legacy compiler
  aliases are rewritten, so `<specfile>.json %clang` reports `^clang` while the same
  edge on an abstract root would have become `llvm`.
- Parsing a git-version literal only stores a lookup; the repository access happens
  when the spec is *formatted*, so `str(Spec("git-test@git.foo/bar"))` can raise an
  `AttributeError` out of `spack.util.hash` for a package with no `git` attribute.

## Spec graph and algebra (`spack/spec.py`)

- `_dup`'s `changed` result is an **`and`-chain over all attribute differences**
  (almost certainly meant to be `or`): `Spec("a@1")._dup(Spec("a@2"))` returns
  `False`.
- `copy(deps=<int depflag>)` silently copies **all** deptypes: only tuple/list/str
  arguments go through `dt.canonicalize`.
- `Spec.copy` hardcodes `Spec.__new__(Spec)`, so `_ImmutableSpec.copy()` yields a
  plain mutable `Spec`.
- `_cmp_iter`'s BFS traversal seeds `visited={0}` — a literal int sentinel that no
  `id()` ever equals, i.e. an intentionally empty visited set in disguise.
- `_format_dependencies` temporarily renames `edge.spec.name` for legacy compiler
  aliases and restores it in a `finally`, masking exceptions raised mid-format; the
  recursion also resets `color` to False.
- `dependencies(deptype=None)` raises `ValueError` (from `canonicalize(None)`)
  rather than meaning "all deptypes".
- Compiler flags never make specs disjoint: `FlagMap`s always intersect
  (`_disjoint_node_attributes_reason` skips them), so `cflags=-O2` and `cflags=-O3`
  intersect and constrain to their union.
- `UnsatisfiableDependencySpecError(other, self)` — the provided/required argument
  order is reversed relative to the sibling node errors.
- EdgeMap keys go stale by design: a merge that names an anonymous node leaves the
  edge filed under `""` in its dependents' maps; all scans read `edge.spec.name`,
  not the bucket key.
- Edge `__getstate__` emits the `(None, slots_dict)` pair characteristic of
  slots-only pickling; anything reading pickled state must accept that shape.

## Solver fact generation (`spack/solver/core.py`)

- `AspFunction.__str__`'s fallback branch quotes `str(arg)` **without escaping** —
  bools and foreign objects can inject unescaped quotes/backslashes into the ASP
  text, unlike genuine strings which are escaped.
