# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Algebraic properties of ``Spec.satisfies``, ``Spec.intersects`` and ``Spec.constrain``.

A spec denotes the set of concrete specs it can be concretized to, a concrete spec being a
singleton. That makes the three operations set operations::

    a.satisfies(b)     a ⊆ b
    a.intersects(b)    a ∩ b ≠ ∅
    a.constrain(b)     a := a ∩ b

The sets a spec can denote are closed under ∩ but not under ∪, so ordered by ⊆ they are a
meet-semilattice with ∩ as the meet. Its top is the anonymous spec, which constrains nothing. It
has no bottom: no spec denotes ∅, so ``constrain`` raises instead of producing one, and
``spack.spec.meet`` adjoins one by returning None.

The laws that follow are checked on hand-picked cases, each saying what one of them means on an
example a reader can follow.

Some laws do not hold. Each gap is pinned by its own test, named for what it demonstrates. Some
gaps are what a spec means and say so; the rest are defects that should start failing the day
they are fixed.
"""

import pytest

import spack.deptypes as dt
import spack.error
from spack.spec import Spec, meet
from spack.spec_parser import _parse_toolchain_config


@pytest.mark.parametrize(
    "spec_str",
    [
        "pkg-a",
        "builtin_mock.pkg-a",
        "pkg-a@1:3",
        "pkg-a+foo",
        "pkg-a foo=bar,baz",
        "pkg-a cflags=-O2",
        "pkg-a target=x86_64:",
        "pkg-a/abcdef",
        "pkg-a patches=abcdef",
        "pkg-a ^pkg-b@1 ^pkg-c",
        "pkg-a %pkg-b",
        "mpi",
        "pkg-a ^[virtuals=mpi] mpich",
        "pkg-a platform=* os=* target=*",
    ],
)
def test_satisfies_is_reflexive(spec_str, mock_packages):
    """Every spec is inside itself."""
    spec = Spec(spec_str)
    assert spec.satisfies(spec)


@pytest.mark.parametrize(
    "spec_str",
    [
        "pkg-a",
        "builtin_mock.pkg-a",
        "pkg-a@1:3",
        "pkg-a+foo",
        "pkg-a foo=bar,baz",
        "pkg-a cflags=-O2",
        "pkg-a target=x86_64:",
        "pkg-a/abcdef",
        "pkg-a patches=abcdef",
        "pkg-a ^pkg-b@1 ^pkg-c",
        "pkg-a %pkg-b",
        "mpi",
        "pkg-a ^[virtuals=mpi] mpich",
        "pkg-a platform=* os=* target=*",
    ],
)
def test_intersects_is_reflexive(spec_str, mock_packages):
    """Every spec overlaps itself."""
    spec = Spec(spec_str)
    assert spec.intersects(spec)


@pytest.mark.parametrize(
    "lhs_str,rhs_str",
    [
        # Disjoint and overlapping names
        ("pkg-a", "pkg-b"),
        ("pkg-a", "builtin_mock.pkg-a"),
        # Versions
        ("pkg-a@1:3", "pkg-a@2"),
        ("pkg-a@1:3", "pkg-a@5"),
        # Variants
        ("pkg-a+foo", "pkg-a~foo"),
        ("pkg-a foo=bar", "pkg-a foo=baz"),
        # Compiler flags
        ("pkg-a cflags=-O2", "pkg-a cflags=-g"),
        # Architecture, including two ranges of the same family
        ("pkg-a target=haswell", "pkg-a target=x86_64:"),
        ("pkg-a target=x86_64:", "pkg-a target=:icelake"),
        ("pkg-a target=x86_64:", "pkg-a target=ppc64le:"),
        # Abstract hashes
        ("pkg-a/abcdef", "pkg-a/abc"),
        ("pkg-a/abcdef", "pkg-a/ffffff"),
        # Dependencies and virtuals
        ("pkg-a ^pkg-b@1", "pkg-a ^pkg-b@2"),
        ("mpi", "pkg-a ^[virtuals=mpi] mpich"),
        ("pkg-a", "mpi"),
    ],
)
def test_intersects_is_symmetric(lhs_str, rhs_str, mock_packages):
    """Whether two specs overlap does not depend on the order they're compared in."""
    lhs, rhs = Spec(lhs_str), Spec(rhs_str)
    assert lhs.intersects(rhs) == rhs.intersects(lhs)


@pytest.mark.parametrize(
    "a_str,b_str,c_str",
    [
        ("pkg-a@2", "pkg-a@1:3", "pkg-a@:5"),
        ("pkg-a foo=bar,baz", "pkg-a foo=bar", "pkg-a"),
        ("pkg-a cflags=-O2", "pkg-a cflags=-O2", "pkg-a"),
        ("pkg-a target=haswell", "pkg-a target=x86_64:", "pkg-a"),
        ("pkg-a/abcdef1234", "pkg-a/abcdef", "pkg-a"),
        ("pkg-a ^pkg-b@1", "pkg-a ^pkg-b@1:3", "pkg-a"),
        ("pkg-a %pkg-b", "pkg-a %pkg-b", "pkg-a"),
    ],
)
def test_satisfies_is_transitive(a_str, b_str, c_str, mock_packages):
    """A spec inside a spec that is itself inside a third is inside the third."""
    a, b, c = Spec(a_str), Spec(b_str), Spec(c_str)
    assert a.satisfies(b)
    assert b.satisfies(c)
    assert a.satisfies(c)


@pytest.mark.parametrize(
    "lhs_str,rhs_str",
    [
        ("pkg-a@1:3", "pkg-a@2"),
        ("pkg-a foo=bar,baz", "pkg-a foo=baz"),
        ("pkg-a cflags=-O2", "pkg-a"),
        ("pkg-a target=haswell", "pkg-a target=x86_64:"),
        ("pkg-a/abcdef", "pkg-a/abc"),
        ("pkg-a", "pkg-a ^pkg-b@1"),
        ("pkg-a ^pkg-b@1 ^pkg-c", "pkg-a"),
        ("pkg-a platform=test", "pkg-a os=*"),
    ],
)
def test_constrain_is_commutative(lhs_str, rhs_str, mock_packages):
    """The intersection does not depend on the order of lhs and rhs."""
    lhs, rhs = Spec(lhs_str), Spec(rhs_str)
    forward = meet(lhs, rhs)
    backward = meet(rhs, lhs)
    assert (forward is None) == (backward is None)
    if forward is not None:
        assert forward.to_dict() == backward.to_dict()


@pytest.mark.parametrize(
    "a_str,b_str,c_str",
    [
        ("pkg-a@1:3", "pkg-a@:2", "pkg-a@2"),
        ("pkg-a foo=bar,baz", "pkg-a foo=baz", "pkg-a"),
        ("pkg-a cflags=-O2", "pkg-a", "pkg-a"),
        ("pkg-a target=haswell", "pkg-a target=x86_64:", "pkg-a"),
        ("pkg-a", "pkg-a ^pkg-b@1", "pkg-a"),
    ],
)
def test_constrain_is_associative(a_str, b_str, c_str, mock_packages):
    """Intersecting three specs gives the same result whichever two are intersected first."""
    a, b, c = Spec(a_str), Spec(b_str), Spec(c_str)
    ab, bc = meet(a, b), meet(b, c)
    left = meet(ab, c) if ab is not None else None
    right = meet(a, bc) if bc is not None else None
    assert (left is None) == (right is None)
    if left is not None:
        assert left.to_dict() == right.to_dict()


@pytest.mark.parametrize(
    "lhs_str,rhs_str",
    [
        ("pkg-a@2", "pkg-a@1:3"),
        ("pkg-a foo=bar,baz", "pkg-a foo=bar"),
        ("pkg-a target=haswell", "pkg-a target=x86_64:"),
        ("pkg-a target=:icelake", "pkg-a target=x86_64:"),
        ("pkg-a/abcdef", "pkg-a/abc"),
        ("pkg-a ^pkg-b@1", "pkg-a"),
    ],
)
def test_constrain_absorbs_a_satisfied_constraint(lhs_str, rhs_str, mock_packages):
    """A spec already inside another has nothing left to intersect, so the meet is the spec."""
    lhs, rhs = Spec(lhs_str), Spec(rhs_str)
    assert lhs.satisfies(rhs)
    result = meet(lhs, rhs)
    assert result is not None
    assert result.to_dict() == lhs.to_dict()


def test_self_meet_of_parallel_deptype_edges_is_idempotent(mock_packages):
    """Two parallel edges meet an equal pair as themselves. Pairing them by name alone would merge
    build into the link edge and produce an edge neither spec required."""
    s = Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e")
    changed = s.constrain(Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e"))
    assert not changed
    assert s.to_dict() == Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e").to_dict()


@pytest.mark.parametrize(
    "a_str,b_str,c_str",
    [
        ("pkg-a@1:3", "pkg-a@2:5", "pkg-a@2:3"),
        ("pkg-a foo=bar,baz", "pkg-a foo=baz,fee", "pkg-a foo=bar,baz,fee"),
        ("pkg-a target=x86_64:", "pkg-a target=:icelake", "pkg-a target=haswell"),
        ("pkg-a target=cascadelake:", "pkg-a target=cannonlake:", "pkg-a target=icelake"),
        ("pkg-a", "pkg-a ^pkg-b@1", "pkg-a ^pkg-b@1"),
    ],
)
def test_constrain_is_the_greatest_lower_bound(a_str, b_str, c_str, mock_packages):
    """Anything inside both a and b is inside their meet too. That makes the meet the
    intersection, not merely some spec contained in both."""
    a, b, c = Spec(a_str), Spec(b_str), Spec(c_str)
    assert c.satisfies(a)
    assert c.satisfies(b)
    result = meet(a, b)
    assert result is not None
    assert c.satisfies(result)


def test_incomparable_target_bounds_meet_as_a_union_of_ranges(mock_packages):
    """Microarchitectures are ordered by a DAG, not a lattice, so two ranges can have more than one
    minimal common bound. The meet is then a list of ranges."""
    lhs, rhs = Spec("pkg-a target=cascadelake:"), Spec("pkg-a target=cannonlake:")
    forward, backward = meet(lhs, rhs), meet(rhs, lhs)
    assert str(forward.architecture.target) == "icelake:"
    assert forward.to_dict() == backward.to_dict()

    # armv8.6a and neoverse_n1 have two minimal common upper bounds, so the meet is a list
    lhs, rhs = Spec("pkg-a target=armv8.6a:"), Spec("pkg-a target=neoverse_n1:")
    forward, backward = meet(lhs, rhs), meet(rhs, lhs)
    assert str(forward.architecture.target) == "ampere1:,ampere1a:"
    assert forward.to_dict() == backward.to_dict()


@pytest.mark.parametrize(
    "a_str,b_str,c_str",
    [
        ("pkg-a@2", "pkg-a@1:3", "pkg-a"),
        ("pkg-a foo=bar,baz", "pkg-a foo=bar", "pkg-a"),
        ("pkg-a target=haswell", "pkg-a target=x86_64:", "pkg-a os=debian6"),
    ],
)
def test_constrain_is_monotonic(a_str, b_str, c_str, mock_packages):
    """Narrowing either spec narrows the meet, so a smaller spec cannot produce a larger
    result."""
    a, b, c = Spec(a_str), Spec(b_str), Spec(c_str)
    assert a.satisfies(b)
    meet_ac, meet_bc = meet(a, c), meet(b, c)
    if meet_ac is None:
        return
    assert meet_bc is not None
    assert meet_ac.satisfies(meet_bc)


def test_a_second_direct_provider_under_an_undecided_condition_is_fine(mock_packages):
    """A node takes each virtual from exactly one of its direct dependencies. A conditional direct
    provider contradicts an unconditional one only where its condition holds, so while the
    condition is undecided the two intersect."""
    lhs = Spec("pkg-a %[virtuals=c] gcc")
    rhs = Spec("pkg-a %[when='+foo' virtuals=c] llvm")
    assert lhs.intersects(rhs)
    assert rhs.intersects(lhs)

    # satisfies implies intersects, and a spec falsifying the condition satisfies both
    witness = Spec("pkg-a ~foo %[virtuals=c] gcc")
    assert witness.satisfies(lhs) and witness.satisfies(rhs)
    assert witness.intersects(rhs)

    satisfied = Spec("pkg-a +foo %[virtuals=c] gcc")
    assert not satisfied.intersects(rhs)
    assert not rhs.intersects(satisfied)
    assert meet(satisfied, rhs) is None


# Where the laws above do not hold. Some cases are what a spec means and say so; the rest are
# defects that should start failing the day they are fixed.


def test_a_propagated_variant_follows_non_contradiction(mock_packages):
    """A propagating variant constrains every transitive dependency that has it, which satisfies
    cannot check structurally. It falls back to non-contradiction: a spec with no opinion on the
    variant satisfies the propagation, which breaks transitivity."""
    assert Spec("pkg-a~foo").satisfies("pkg-a")
    assert Spec("pkg-a").satisfies("pkg-a++foo")
    assert not Spec("pkg-a~foo").satisfies("pkg-a++foo")


def test_flag_order_is_significant_so_the_meet_is_not_commutative(mock_packages):
    """Flag order is significant to the build, so flags are a sequence, not a set. cflags='-O2 -g'
    and cflags='-g -O2' are two states, and the union that merges them does not commute."""
    lhs, rhs = Spec("pkg-a cflags=-O2"), Spec("pkg-a cflags=-g")
    forward = meet(lhs, rhs)
    backward = meet(rhs, lhs)
    assert forward.to_dict() != backward.to_dict()


# Laws that hold only because of one decision in the merge, pinned on the state that stops
# satisfying them the day it is lost.


def test_a_virtual_edge_constraining_a_version_stays_unfused_but_still_satisfies(mock_packages):
    """A version on an edge naming a virtual bounds the virtual, not its provider, and a node has
    nowhere to record that, so the edge stays beside the provider edge. Satisfies still matches
    one against the other directly."""
    narrower = Spec("pkg-a ^[virtuals=mpi] mpich+debug")
    wider = Spec("pkg-a ^mpi+debug")
    assert narrower.satisfies(wider)

    third = Spec("pkg-a ^mpi@3")
    narrowed, widened = meet(narrower, third), meet(wider, third)
    assert widened is not None
    assert narrowed is not None
    assert len(narrowed.edges_to_dependencies()) == 2  # '^mpi@3' and '^[virtuals=mpi] mpich+debug'
    assert narrowed.satisfies(widened)

    # constrain() and satisfies() share the implication check that pairs edges, so a spec left
    # with two unfused edges still satisfies itself
    assert narrowed.satisfies(narrowed)
    assert narrowed.constrain(narrowed) is False


def test_implication_does_not_fuse_across_a_nested_dependency(mock_packages):
    """A bare edge to pkg-b does not imply one that itself depends on pkg-e: they are independent
    requirements, and fusing them demands both of a single edge. Kept apart, they leave every merge
    order at the same state."""
    a = Spec("^pkg-b %pkg-e")
    b = Spec("^[deptypes=build] pkg-b ^[deptypes=link] pkg-b")
    c = Spec("%pkg-b ^pkg-b")

    ab, bc = meet(a, b), meet(b, c)
    left, right = meet(ab, c), meet(a, bc)

    assert left is not None and right is not None
    assert left.to_dict() == right.to_dict()
    assert left.satisfies(a)
    assert left.satisfies(b)
    assert left.satisfies(c)


def test_copy_keeps_a_redundant_parallel_edge_and_its_subtree(mock_packages):
    """A structural copy reproduces every edge as it is. ``_dup_deps`` builds each edge directly,
    since replaying them through ``add_dependency_edge`` would discard the pkg-b@1: edge before its
    child pkg-e is attached."""
    original = Spec("pkg-a ^[deptypes=link] pkg-b@1")
    dep = Spec("pkg-b@1:")
    dep._add_dependency(Spec("pkg-e"), depflag=dt.LINK, virtuals=())
    original._add_dependency(dep, depflag=dt.LINK, virtuals=())

    copy = original.copy()

    assert len(copy.edges_to_dependencies(name="pkg-b")) == 2
    assert any(s.name == "pkg-e" for s in copy.traverse())
    assert copy.to_dict() == original.to_dict()


def test_one_target_range_is_one_canonical_state(mock_packages):
    """':icelake' and 'x86_64:icelake' denote the same range, since x86_64 is the family root.
    Ranges are stored canonicalized, so the two are one state with one hash."""
    long, short = Spec("pkg-a target=x86_64:icelake"), Spec("pkg-a target=:icelake")
    assert long.to_dict() == short.to_dict()
    assert long.dag_hash() == short.dag_hash()

    lhs, rhs = Spec("pkg-a target=:icelake"), Spec("pkg-a target=x86_64:")
    forward = meet(lhs, rhs)
    backward = meet(rhs, lhs)
    assert forward.to_dict() == backward.to_dict()
    assert str(forward.architecture.target) == ":icelake"


def test_a_target_range_inside_another_one_is_dropped_from_the_list(mock_packages):
    """A list of ranges denotes their union, so a range inside another adds nothing to it and is
    dropped, leaving one canonical state for that union."""
    assert (
        Spec("pkg-a target=cannonlake:,icelake:").to_dict()
        == Spec("pkg-a target=cannonlake:").to_dict()
    )


def test_a_conditional_edge_merges_the_same_from_either_edge_order(mock_packages):
    """A conditional edge is canonicalized against the finished node, so two spec strings differing
    only in the order of their edges parse and meet to the same state. Here ``%pkg-e@1``
    falsifies the condition, so the conditional edge constrains nothing and is dropped."""
    o1 = Spec("pkg-a %[when='%pkg-e@2'] pkg-c@1 %pkg-e@1")
    o2 = Spec("pkg-a %pkg-e@1 %[when='%pkg-e@2'] pkg-c@1")
    assert o1.to_dict() == o2.to_dict() == Spec("pkg-a %pkg-e@1").to_dict()

    forward, backward = meet(Spec("pkg-a"), o1), meet(Spec("pkg-a"), o2)
    assert forward is not None and backward is not None
    assert forward.to_dict() == backward.to_dict() == o1.to_dict()


def test_an_edge_the_merge_falsifies_is_pruned_from_both_sides(mock_packages):
    """An edge whose condition cannot hold for the merged node constrains nothing, so it is
    deleted whichever spec it came from."""
    x, y = Spec("pkg-a %pkg-e@1"), Spec("pkg-a %[when='%pkg-e@2'] pkg-c@1")
    assert x.satisfies(y)
    forward, backward = meet(x, y), meet(y, x)
    assert forward.to_dict() == x.to_dict()
    assert backward.to_dict() == x.to_dict()


def test_parse_and_constrain_canonicalize_a_condition_the_same_way(mock_packages):
    """A statically decided condition is canonicalized whichever path instantiates the spec:
    a falsified edge is dropped and an edge with a satisfied condition is made unconditional,
    by the parser and by constrain alike."""
    falsified = Spec("pkg-a ^[when='+foo'] pkg-b")
    falsified.constrain("~foo")
    assert falsified.to_dict() == Spec("pkg-a ~foo ^[when='+foo'] pkg-b").to_dict()
    assert falsified.to_dict() == Spec("pkg-a ~foo").to_dict()

    satisfied = Spec("pkg-a ^[when='+foo'] pkg-b")
    satisfied.constrain("+foo")
    assert satisfied.to_dict() == Spec("pkg-a +foo ^[when='+foo'] pkg-b").to_dict()
    assert satisfied.to_dict() == Spec("pkg-a +foo ^pkg-b").to_dict()


def test_a_satisfied_condition_merges_the_edge(mock_packages):
    """Once the node satisfies its condition, a conditional direct edge is the same dependency as
    an unconditional one to the same name: the pair merges into one edge, or is refused when
    the children are disjoint, instead of standing as parallel edges that describe the empty
    set."""
    merged = Spec("pkg-a +foo %[when='+foo'] pkg-b@1 %pkg-b@:2")
    assert merged.to_dict() == Spec("pkg-a +foo %pkg-b@1").to_dict()

    with pytest.raises(spack.error.SpecSyntaxError):
        Spec("pkg-a +foo %[when='+foo'] pkg-b@1 %pkg-b@2")

    assert not Spec("pkg-a %[when='+foo'] pkg-b@1").intersects("pkg-a +foo %pkg-b@2")


def test_an_edge_conditional_on_dependencies_stays_conditional(mock_packages):
    """A condition that constrains dependencies is never decided by the node, so the condition
    stays even where the graph it hangs off satisfies it."""
    spec = Spec("pkg-a %pkg-e ^[when='%pkg-e'] pkg-b")
    assert len(spec.edges_to_dependencies(name="pkg-b")) == 1
    assert spec.edges_to_dependencies(name="pkg-b")[0].when == Spec("%pkg-e")


def test_toolchain_config_entries_commute(mock_packages):
    """A toolchain config is folded into one spec entry by entry, so the result must not depend
    on the order the YAML lists them in."""
    a = {"spec": "%pkg-c@1", "when": "%pkg-e@2"}
    b = {"spec": "%pkg-e@1"}
    forward, backward = _parse_toolchain_config([a, b]), _parse_toolchain_config([b, a])
    assert forward.to_dict() == backward.to_dict()


def test_same_dep_under_one_undecided_condition_meets_as_parallel_edges(mock_packages):
    """Two direct edges to one name under equal conditions are one dependency only where the node
    satisfies the condition. While it is undecided, the specs intersect and the meet keeps both
    edges."""
    lhs = Spec("pkg-a %[when='+foo'] pkg-b@1")
    rhs = Spec("pkg-a %[when='+foo'] pkg-b@2")
    assert lhs.intersects(rhs) and rhs.intersects(lhs)

    result = meet(lhs, rhs)
    assert result is not None
    assert len(result.edges_to_dependencies(name="pkg-b")) == 2
    assert result.satisfies(lhs) and result.satisfies(rhs)
    assert Spec(str(result)).to_dict() == result.to_dict()
    assert Spec.from_dict(result.to_dict()).to_dict() == result.to_dict()

    # satisfying the condition on either side closes the escape and the pair conflicts again
    assert not Spec("pkg-a +foo %[when='+foo'] pkg-b@1").intersects(rhs)
    assert meet(Spec("pkg-a +foo"), result) is None

    # the meet is then associative around a spec that closes the condition off
    negated = Spec("pkg-a ~foo")
    left = meet(meet(negated, lhs), rhs)
    right = meet(negated, result)
    assert left is not None and right is not None
    assert left.to_dict() == right.to_dict() == negated.to_dict()


def test_same_dep_under_a_dep_constraining_condition_meets_as_parallel_edges(mock_packages):
    """A condition that constrains dependencies is never decided by the node, so equal-condition
    direct edges to one name stay parallel: the spec denotes the concretizations that falsify
    the condition, and the meet keeps that escape open."""
    lhs = Spec("pkg-a %[when='%pkg-e@2'] pkg-b@1")
    rhs = Spec("pkg-a %[when='%pkg-e@2'] pkg-b@2")
    assert lhs.intersects(rhs) and rhs.intersects(lhs)

    result = meet(lhs, rhs)
    assert result is not None
    assert len(result.edges_to_dependencies(name="pkg-b")) == 2
    assert result.satisfies(lhs) and result.satisfies(rhs)
    assert Spec(str(result)).to_dict() == result.to_dict()

    # falsifying the condition prunes both edges
    closed = meet(result, Spec("pkg-a %pkg-e@1"))
    assert closed is not None
    assert closed.to_dict() == Spec("pkg-a %pkg-e@1").to_dict()


def test_a_jointly_empty_group_of_direct_edges_blocks_the_meet(mock_packages):
    """Three children can intersect pairwise while their joint intersection is empty: @1:2,
    @2:3 and @1,3 share no version. Once the merged node decides their shared condition, all
    three edges must be one node, so the specs are disjoint, and a pre-check reasoning in
    pairs would miss it."""
    x = Spec("pkg-a %[when='+foo'] pkg-b@1:2 %[when='+foo'] pkg-b@2:3")
    y = Spec("pkg-a+foo %pkg-b@1,3")
    assert not x.intersects(y)
    assert not y.intersects(x)
    assert meet(x, y) is None
    assert meet(y, x) is None

    # a failed constrain leaves self untouched
    z = x.copy()
    with pytest.raises(spack.error.SpecError):
        z.constrain(y)
    assert z.to_dict() == x.to_dict()

    # the group can also live inside one spec, with the other contributing only the node
    triple = Spec(
        "pkg-a %[when='+foo'] pkg-b@1:2 %[when='+foo'] pkg-b@2:3 %[when='+foo'] pkg-b@1,3"
    )
    assert not triple.intersects(Spec("pkg-a+foo"))
    assert meet(triple, Spec("pkg-a+foo")) is None


def test_a_jointly_nonempty_group_of_direct_edges_merges(mock_packages):
    """The running merge of a group narrows the child by every edge: @1:2, @2:3 and @2:4
    meet at @2, and the edges are made unconditional."""
    x = Spec("pkg-a %[when='+foo'] pkg-b@1:2 %[when='+foo'] pkg-b@2:3")
    y = Spec("pkg-a+foo %pkg-b@2:4")
    expected = Spec("pkg-a+foo %pkg-b@2")
    forward, backward = meet(x, y), meet(y, x)
    assert forward is not None and backward is not None
    assert forward.to_dict() == backward.to_dict() == expected.to_dict()


def test_provider_pair_under_a_dep_constraining_condition_stays_parallel(mock_packages):
    """The same escape for two direct providers of one virtual: with their shared condition
    undecided by the node, the pair stays parallel rather than conflicting."""
    lhs = Spec("pkg-a %[when='%pkg-e@2' virtuals=c] gcc")
    rhs = Spec("pkg-a %[when='%pkg-e@2' virtuals=c] llvm")
    assert lhs.intersects(rhs) and rhs.intersects(lhs)

    result = meet(lhs, rhs)
    assert result is not None
    assert len(result.edges_to_dependencies()) == 2

    closed = meet(result, Spec("pkg-a %pkg-e@1"))
    assert closed is not None
    assert closed.to_dict() == Spec("pkg-a %pkg-e@1").to_dict()


def test_a_provider_pair_whose_conditions_must_hold_blocks_the_meet(mock_packages):
    """The provider pair lives inside a single spec, legally: its condition is undecided there.
    The other spec's node decides it, so the conflict shows only among the edges of one
    spec."""
    node = Spec("+foo")
    providers = Spec("%[when='+foo' virtuals=c] gcc %[virtuals=c] llvm")
    assert not node.intersects(providers)
    assert not providers.intersects(node)
    assert meet(node, providers) is None
    assert meet(providers, node) is None

    # undecided, the same spec meets a plain node unharmed
    result = meet(Spec("pkg-a"), Spec("pkg-a %[when='+foo' virtuals=c] gcc %[virtuals=c] llvm"))
    assert result is not None
    assert len(result.edges_to_dependencies()) == 2
