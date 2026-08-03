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


def test_self_meet_of_parallel_deptype_edges_is_idempotent(mock_packages):
    """Two parallel edges meet an equal pair as themselves. Pairing them by name alone would merge
    build into the link edge and produce an edge neither spec required."""
    s = Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e")
    changed = s.constrain(Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e"))
    assert not changed
    assert s.to_dict() == Spec("pkg-a ^[deptypes=build] pkg-e ^[deptypes=link] pkg-e").to_dict()


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
