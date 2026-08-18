# Branch state after the rebase onto develop (2026-08-18)

Status notes for `hs/test/property-based-tests` / `hs/test/spec-algebra-fuzz` (PR 52801).
The replay recipe this file used to hold was executed on 2026-08-18, when the extraction
PRs #52876, #52893, #52905, #52906, #52907 and #52910 had all merged.

## State

- The branch sits on upstream/develop plus the four commits of
  `hs/fix/arch-target-ranges` (PR 52856), the one extraction PR still open, rebased onto
  develop and carried at the bottom. Once 52856 merges, a plain rebase onto develop drops
  those four commits and the branch carries only its own content.
- The 20 pre-rebase commits replayed in order. Commits covered by the merged PRs shrank to
  their test extras or fell away; where develop and the branch had diverged on shape or
  naming, develop's version was kept and the branch's machinery adapted to it (`_implies_edge`
  regained the propagation guard, `_add_or_merge_edge` kept develop's discard/replace flow,
  `Spec._merge` ports the concrete-rhs replacement of ca4825110d2).
- Gate: a reference merge of the new base into the old tip `9a7e4f24fc7` produced the same
  tree as the rebased tip, apart from these two root docs. The old tip is kept at
  `backup-property-based-tests-20260818`.
- `hs/test/spec-algebra-fuzz` (the PR head) points at the last code commit: the harness
  scripts under `lib/spack/spack/test/` and these docs stay out of the PR.

See `bug-ledger.md` for the bug counts per PR.
