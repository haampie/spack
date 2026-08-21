# Concretizer fuzz findings, 2026-08-21/22

Method: `lib/spack/spack/test/concretize_fuzz.py` draws seeded random inputs against the real
builtin repo -- a root package decorated with version, variant, flag, target, dependency and
virtual constraints sampled from the packages' own metadata, plus randomized `packages:` config
(`require` in plain, `any_of`, `one_of` and conditional forms, externals with `buildable:
false`), concretizer modes (`unify` together and when_possible, `duplicates: none|minimal`,
`targets: granularity: generic`) -- concretizes fresh, and then checks the output DAG against
every input the solver saw: the input literals, the injected config, and each package's
`depends_on` (presence, constraint, deptype), `conflicts`, `requires`, variant definitions
(presence, activation, allowed values, stickiness), declared versions, and provider
legitimacy. About 1050 randomized cases ran, plus targeted probes where random inputs pointed
at a seam. Solver output held up on almost every axis; the bugs cluster in two places:
propagation, and constraint features that the requirement machinery's condition matching
cannot see.

## Bugs

**1. Propagated compiler flags silently skip dependencies missing one of the source's
languages.** `spack solve --fresh "mpich+fortran cflags=='-O2'"` puts `-O2` on mpich and on no
dependency at all; `mpich~fortran cflags=='-O2'` propagates to zlib-ng, hwloc and ncurses but
still not to libxml2 -- in the same solve -- because libxml2 uses only C while mpich also uses
C++. The docs promise `spack install libdwarf cppflags=="-g"` installs libelf with `-g`, and
the unit tests promise propagation gated on "same compiler"; neither warns that a C++-or-Fortran
using root stops propagating *C* flags to C-only dependencies whose C compiler is identical.
The cause is the `propagated_flag` rule (`concretize.lp:1778-1786`): its conditional literal
`node_compiler(Target, C, L) : node_compiler(Source, C, L)` demands the target support every
(compiler, language) pair of the source, rather than the pair belonging to the flag's language.
Any mixed-language root therefore fails to propagate flags to leaf libraries, which is the
common case (kallisto, sbml and mpich all hit it in the fuzz runs). Reproduces on develop.

**2. `one_of` requirements with a propagation-marked member: output satisfies two members of an
exactly-one group.** With

```yaml
packages:
  zlib-ng:
    require:
    - one_of: ["@2.0.0:", "@2.0.0: ++new_strategies"]
```

`spack solve --fresh zlib-ng` returns `zlib-ng@2.3.3+new_strategies`, which satisfies both
members; the documented semantics is "one and only one". Internally the solver treats
`++new_strategies` as unmet (propagation is not active), while `Spec.satisfies` treats the
marker as a preference and matches on `+new_strategies`. Found three times independently by
the fuzzer (perl `++cpanm`, zlib-ng `++new_strategies`, ninja `++re2c`).

**3. `one_of` with compiler-flag members: exact-group matching vs subset satisfaction.** With
`require: [{one_of: ["cflags=-O2", "cflags='-O2 -g'"]}]` on zlib-ng, the input
`zlib-ng cflags='-O2 -g'` concretizes successfully and the output satisfies *both* members
(`satisfies` matches flag lists as subsets). Exactly-one semantics requires this input to be
unsatisfiable. The solver's condition matching for flags is exact-group, so it believes only
the second member is met.

**4. `one_of` with `%` members: overlapping direct-dependency constraints both satisfied.**
With `require: [{one_of: ["%apple-clang", "%apple-clang@17:"]}]` on zlib-ng, the solve returns
a spec compiled with apple-clang@17.0.0 that satisfies both members. Since every spec matching
the second member matches the first, the group is unsatisfiable under exactly-one semantics,
and the solver should say so instead of returning a doubly-matching answer. Version-range
overlap between plain version members is detected correctly; the `%`-edge variant of the same
overlap is not.

**5. A requirement whose `when` condition is a compiler flag is silently not applied.** With

```yaml
packages:
  zlib-ng:
    require:
    - spec: "~pic"
      when: "cflags=-O2"
```

`spack solve --fresh "zlib-ng cflags=-O2"` returns `zlib-ng cflags=-O2 +pic`: the output
satisfies the `when` condition exactly (its cflags are precisely `-O2`) but not the required
spec. The same requirement gated on `when: "%apple-clang"` is applied correctly, so the gap is
specific to flag-valued conditions -- the same condition-matching seam as bugs 2-4, here
causing an unconditional violation of the configured requirement rather than a miscounted
group.

**6. A propagation marker in a plain requirement makes the solve non-terminating.** With
`packages: {libxml2: {require: ["~~shared"]}}`, `spack solve --fresh gettext` did not return
within 400 seconds; the same solve without the requirement takes about 10. There is no
diagnostic, just clingo grinding, so from the user's perspective concretization hangs.

**7. Internal error instead of a diagnosis for a forced non-provider.**
`spack solve "hypre~fortran ^[virtuals=mpi] zlib-ng"` fails with "Spack concretizer internal
error. Please submit a bug report ..." -- the error tree machinery finds no cause for the
unsatisfiability, and by the message's own definition that state is a bug. A plain "zlib-ng
does not provide mpi" is derivable from the input alone.

**8. Internal error for unified roots with conflicting targets.** Concretizing
`zlib-ng target=m1` and `zlib-ng target=m3` together (`unify: true`) fails with the same
"internal error, please submit a bug report" instead of reporting that the two roots cannot
unify. The when_possible mode handles the analogous version conflict fine by splitting.

**9. Requirements that only match deprecated versions report "no known version".** With
openssl 3.0.18 declared but `deprecated=True`, `packages:openssl:require: ["@3.0.18"]` raises
`ConfigError: Version requirement 3.0.18 on openssl ... cannot match any known version from
package.py or externals`. The version is known; the blocker is deprecation
(`config:deprecated: true` makes the same input work), and the message sends the user hunting
in the wrong place.

**10. The `^c` rejection message contradicts the implemented rule.** `spack solve "zlib-ng ^c"`
fails with "c is not a direct 'build' or 'test' dependency, or transitive 'link' or 'run'
dependency of any root" although c *is* a direct build dependency of the root; what the
implementation actually requires is that the `^` spec resolve against transitive link/run
nodes. The flip side: `kallisto ^c@2021.11` is *accepted*, because kallisto's DAG happens to
contain tree-sitter -- six levels deep, via mvapich-plus, rpm, gnupg, pinentry and emacs --
which has c as a build+run dependency, and the user's constraint silently binds to
tree-sitter's runtime compiler and is vacuously satisfied by apple-clang's unversioned
`provides("c")`. The same constraint shape is unsat or sat depending on whether some
grandchild runs a compiler at runtime.

## Perplexing concretizations

A fresh `kallisto` (a sequence aligner; three direct dependencies) resolves mpi to
mvapich-plus, whose closure drags in rpm, gnupg, pinentry, emacs, tree-sitter, lua, rust,
node-js and llvm -- over eighty nodes, several of them full toolchains, for a package that
needs hdf5 and zlib. Besides the cost, this indirectly changes the meaning of user inputs:
bug 9's `^c` binding exists only because of this closure.

Requiring `~shared` (or any non-default variant) on openblas makes `hypre` switch its
blas/lapack provider to netlib-lapack: `require` on a package only binds if the package
appears, and the default-variant optimization then prefers the provider nobody touched. Legal,
but a require intended to pin down openblas silently removes it instead.

`packages:zlib-ng:require: ["^cmake@3.31:"]` looks like it should be unsatisfiable (zlib-ng
does not depend on cmake) but is satisfied by flipping `build_system=cmake`, which activates a
conditional `depends_on("cmake")`. The requirement machinery composing with conditional
dependencies is impressive and correct -- and thoroughly surprising.

## What held

Negative results worth recording: exactly-one and any-of over version, variant, target-range
and `^`-dependency members (for overlapping target members the solver even degrades the root
to armv8.5a to keep the count at one), requirement `when` conditions gated on `%compiler`,
disjoint-set variant groups (including two requirements whose union violates disjointness),
requirements enforced against reuse candidates, `buildable: false` with non-matching
externals, conflicts and requires directives including `%compiler` forms, conditional
variants and conditional dependencies (including requirement-triggered build-system flips),
sticky variants, deptype coverage of declared edges, provider legitimacy of every virtual
edge, generic-target granularity, together/when_possible unification, and `duplicates:
none|minimal` all came back clean across roughly 1050 randomized cases. The input literal was
satisfied by the output in every SAT case.

## Harness

```
python lib/spack/spack/test/concretize_fuzz.py --seeds 0:100 --jsonl out.jsonl [-v]
```

Each seed is one deterministic case (generation, config, mode). A case forks a child process
bounded by `--wall-limit` so runaway groundings cannot stall the sweep; `--timeout` bounds
clingo itself. Findings print as `!!` lines and land in the JSONL log with the full input for
replay. The script is standalone and intentionally not collected by pytest (real repo, real
config, minutes per case).
