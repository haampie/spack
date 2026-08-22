// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Pure-Rust port of Spack's architecture-target range algebra: the free helper functions in
//! `lib/spack/spack/spec.py` around `ArchSpec` and the target dimension of the `ArchSpec`
//! operations. The Python implementation is the specification; canonical strings produced here
//! are byte-identical to Python's. Platform and operating system are plain strings handled by
//! the binding layer; only the target dimension lives here.
//!
//! The archspec table (`archspec.cpu.TARGETS`) becomes a [`TargetGraph`], built once by the
//! binding layer from `(name, parents, vendor)` records via [`TargetGraph::from_records`].
//! A microarchitecture is a [`Target`]: either a node of the graph or, mirroring
//! `archspec.cpu.generic_microarchitecture`, an unknown name with no parents and vendor
//! `generic`. Within one graph names are unique, so equality of targets is name equality; the
//! partial order is ancestor-set inclusion (`Microarchitecture.__lt__`), under which an unknown
//! name is incomparable to everything but itself. A target *expression* (the `ArchSpec.target`
//! state: a comma-separated union of names and `lo:hi`/`:hi`/`lo:`/`:` intervals) is a
//! [`TargetExpr`], stored in canonical form exactly like `_make_microarchitecture` stores it.
//!
//! # Python name → Rust name
//!
//! | Python (`spack.spec`)                    | Rust                                            |
//! |------------------------------------------|-------------------------------------------------|
//! | `archspec.cpu.TARGETS`                   | [`TargetGraph::from_records`]                   |
//! | `archspec.cpu.Microarchitecture`         | [`Target`]                                      |
//! | `archspec.cpu.generic_microarchitecture` | [`Target::Unknown`] (made by `make_target`)     |
//! | `_make_microarchitecture` (lookup)       | [`TargetGraph::make_target`]                    |
//! | `_make_microarchitecture` (stored name)  | [`TargetExpr::parse`]                           |
//! | `_closed_interval_targets`               | [`TargetGraph::closed_interval_targets`]        |
//! | `_decompose_target_set`                  | [`TargetGraph::decompose_target_set`]           |
//! | `_canonical_target_range`                | [`TargetGraph::canonical_target_range`]         |
//! | `_satisfies_target_range`                | [`TargetGraph::satisfies_target_range`]         |
//! | `_covered_by_target_list`                | [`TargetGraph::covered_by_target_list`]         |
//! | `_parse_target_range`                    | [`TargetGraph::parse_target_range`]             |
//! | `_minimal_upper_bounds`                  | [`TargetGraph::minimal_upper_bounds`]           |
//! | `_maximal_lower_bounds`                  | [`TargetGraph::maximal_lower_bounds`]           |
//! | `ArchSpec._target_satisfies`             | [`TargetGraph::target_satisfies`]               |
//! | `ArchSpec._target_intersects`            | [`TargetGraph::target_intersects`]              |
//! | `ArchSpec._target_intersection`          | [`TargetGraph::target_intersection`]            |
//! | `ArchSpec._target_constrain`             | [`TargetGraph::target_constrain`]               |
//! | `ArchSpec.target_concrete`               | [`target_concrete`] / [`TargetExpr::is_concrete`] |
//! | `UnsatisfiableArchitectureSpecError`     | [`UnsatisfiableTarget`]                         |
//!
//! Python memoizes `_minimal_upper_bounds`/`_maximal_lower_bounds` per pair; here the
//! whole-table scans behind them are cached inside the graph under a mutex, so a shared
//! `TargetGraph` stays `Sync`.
//!
//! Known deviations from Python, none observable in canonical strings:
//!
//! * `_maximal_lower_bounds` iterates a Python `set` of targets, so when two upper bounds have
//!   several maximal common lower bounds the order of the resulting list, and hence of the raw
//!   `_target_intersection` output, depends on the interpreter's hash seed. Here both bound
//!   functions return table order (graph insertion order, which for the archspec JSON matches
//!   `TARGETS` insertion order). Canonical forms sort, so `target_constrain` and every stored
//!   expression are byte-identical regardless.
//! * `Microarchitecture.__eq__` also compares vendor, features, parents, compilers, generation
//!   and cpu_part. Within one graph names are unique and unknown names never shadow table
//!   names (`make_target` resolves table names first), so name equality is equivalent here.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Mutex;

/// A microarchitecture: a node of a [`TargetGraph`] or an unknown generic name
/// (`generic_microarchitecture` in archspec: no parents, vendor `generic`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Known(usize),
    Unknown(String),
}

/// Error building a [`TargetGraph`] from records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetGraphError {
    DuplicateName(String),
    UnknownParent {
        child: String,
        parent: String,
    },
    Cycle(String),
    /// A node reaches more than one root, so it has no unique family
    /// (archspec asserts this never happens).
    AmbiguousFamily {
        name: String,
        roots: Vec<String>,
    },
}

impl fmt::Display for TargetGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetGraphError::DuplicateName(name) => write!(f, "duplicate target name {name:?}"),
            TargetGraphError::UnknownParent { child, parent } => {
                write!(f, "target {child:?} has unknown parent {parent:?}")
            }
            TargetGraphError::Cycle(name) => write!(f, "target {name:?} is part of a cycle"),
            TargetGraphError::AmbiguousFamily { name, roots } => {
                write!(f, "target {name:?} belongs to several families: {roots:?}")
            }
        }
    }
}

impl std::error::Error for TargetGraphError {}

/// The target dimension of `UnsatisfiableArchitectureSpecError`: the two expressions passed to
/// [`TargetGraph::target_constrain`] denote disjoint sets of targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsatisfiableTarget {
    pub lhs: String,
    pub rhs: String,
}

impl fmt::Display for UnsatisfiableTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "target {:?} does not intersect {:?}", self.lhs, self.rhs)
    }
}

impl std::error::Error for UnsatisfiableTarget {}

/// A target constraint expression in the canonical form `_make_microarchitecture` stores:
/// a comma-separated union of names and intervals (`lo:hi`, `:hi`, `lo:`, `:`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetExpr(String);

impl TargetExpr {
    /// Canonicalize an expression string: `*` becomes `:`, and anything with a `:` or `,` goes
    /// through [`TargetGraph::canonical_target_range`]. Mirrors the name normalization of
    /// `_make_microarchitecture`; it never fails, unknown names are kept verbatim.
    pub fn parse(graph: &TargetGraph, s: &str) -> TargetExpr {
        if s == "*" {
            TargetExpr(":".to_string())
        } else if s.contains(':') || s.contains(',') {
            TargetExpr(graph.canonical_target_range(s))
        } else {
            TargetExpr(s.to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `ArchSpec.target_concrete` for a present target: not a range or list.
    pub fn is_concrete(&self) -> bool {
        !self.0.contains(':') && !self.0.contains(',')
    }
}

impl fmt::Display for TargetExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `ArchSpec.target_concrete`: the target is set and is not a range or list.
pub fn target_concrete(target: Option<&TargetExpr>) -> bool {
    target.is_some_and(|t| t.is_concrete())
}

struct Node {
    name: String,
    vendor: String,
    parents: Vec<usize>,
    /// Transitive closure of `parents`, excluding the node itself.
    ancestor_set: HashSet<usize>,
    family: usize,
}

/// The archspec microarchitecture table as a DAG with precomputed ancestor sets.
pub struct TargetGraph {
    nodes: Vec<Node>,
    by_name: HashMap<String, usize>,
    /// Caches for the whole-table scans of `minimal_upper_bounds`/`maximal_lower_bounds`,
    /// keyed by the (order-normalized) node index pair.
    mub_cache: Mutex<HashMap<(usize, usize), Vec<usize>>>,
    mlb_cache: Mutex<HashMap<(usize, usize), Vec<usize>>>,
}

/// `str.partition(":")`: (before, separator found, after), splitting at the first `:`.
fn partition(s: &str) -> (&str, bool, &str) {
    match s.find(':') {
        Some(i) => (&s[..i], true, &s[i + 1..]),
        None => (s, false, ""),
    }
}

impl TargetGraph {
    /// Build the graph from `(name, parent names, vendor)` records. Records may appear in any
    /// order (parents are resolved after all names are known), but the record order is the
    /// table order: it fixes the iteration order of the bound scans, so to reproduce archspec
    /// byte-for-byte feed the records in `TARGETS` insertion order.
    pub fn from_records<I>(records: I) -> Result<TargetGraph, TargetGraphError>
    where
        I: IntoIterator<Item = (String, Vec<String>, String)>,
    {
        let records: Vec<(String, Vec<String>, String)> = records.into_iter().collect();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for (i, (name, _, _)) in records.iter().enumerate() {
            if by_name.insert(name.clone(), i).is_some() {
                return Err(TargetGraphError::DuplicateName(name.clone()));
            }
        }
        let mut parents: Vec<Vec<usize>> = Vec::with_capacity(records.len());
        for (name, parent_names, _) in &records {
            let mut indices = Vec::with_capacity(parent_names.len());
            for parent in parent_names {
                let &idx = by_name
                    .get(parent)
                    .ok_or_else(|| TargetGraphError::UnknownParent {
                        child: name.clone(),
                        parent: parent.clone(),
                    })?;
                indices.push(idx);
            }
            parents.push(indices);
        }

        // Ancestor sets by iterative DFS; state 1 marks "parents pushed", so meeting a
        // state-1 node from below is a cycle.
        let n = records.len();
        let mut ancestors: Vec<Option<HashSet<usize>>> = (0..n).map(|_| None).collect();
        let mut state = vec![0u8; n];
        for start in 0..n {
            if ancestors[start].is_some() {
                continue;
            }
            let mut stack = vec![start];
            while let Some(&node) = stack.last() {
                if ancestors[node].is_some() {
                    stack.pop();
                    continue;
                }
                if state[node] == 1 {
                    let mut set = HashSet::new();
                    for &p in &parents[node] {
                        set.insert(p);
                        set.extend(ancestors[p].as_ref().unwrap().iter().copied());
                    }
                    ancestors[node] = Some(set);
                    stack.pop();
                    continue;
                }
                state[node] = 1;
                for &p in &parents[node] {
                    if ancestors[p].is_none() {
                        if state[p] == 1 {
                            return Err(TargetGraphError::Cycle(records[p].0.clone()));
                        }
                        stack.push(p);
                    }
                }
            }
        }

        // The family is the unique root among the node and its ancestors.
        let mut families = Vec::with_capacity(n);
        for i in 0..n {
            let anc = ancestors[i].as_ref().unwrap();
            let roots: Vec<usize> = std::iter::once(i)
                .chain(anc.iter().copied())
                .filter(|&t| parents[t].is_empty())
                .collect();
            if roots.len() != 1 {
                return Err(TargetGraphError::AmbiguousFamily {
                    name: records[i].0.clone(),
                    roots: roots.iter().map(|&r| records[r].0.clone()).collect(),
                });
            }
            families.push(roots[0]);
        }

        let nodes = records
            .into_iter()
            .zip(parents)
            .zip(ancestors)
            .zip(families)
            .map(
                |((((name, _, vendor), parents), ancestor_set), family)| Node {
                    name,
                    vendor,
                    parents,
                    ancestor_set: ancestor_set.unwrap(),
                    family,
                },
            )
            .collect();
        Ok(TargetGraph {
            nodes,
            by_name,
            mub_cache: Mutex::new(HashMap::new()),
            mlb_cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The table names in table order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().map(|n| n.name.as_str())
    }

    /// `_make_microarchitecture`: normalize the name (`*` to `:`, ranges/lists to canonical
    /// form), then look it up; unknown names become generic targets.
    pub fn make_target(&self, name: &str) -> Target {
        let name = if name == "*" {
            ":".to_string()
        } else if name.contains(':') || name.contains(',') {
            self.canonical_target_range(name)
        } else {
            name.to_string()
        };
        match self.by_name.get(&name) {
            Some(&i) => Target::Known(i),
            None => Target::Unknown(name),
        }
    }

    pub fn name_of<'a>(&'a self, t: &'a Target) -> &'a str {
        match t {
            Target::Known(i) => &self.nodes[*i].name,
            Target::Unknown(s) => s,
        }
    }

    /// `Microarchitecture.parents` by name, in record order; unknown targets have none.
    pub fn parents_of(&self, t: &Target) -> Vec<&str> {
        match t {
            Target::Known(i) => self.nodes[*i]
                .parents
                .iter()
                .map(|&p| self.nodes[p].name.as_str())
                .collect(),
            Target::Unknown(_) => Vec::new(),
        }
    }

    /// `Microarchitecture.vendor`; unknown targets are generic.
    pub fn vendor_of<'a>(&'a self, t: &'a Target) -> &'a str {
        match t {
            Target::Known(i) => &self.nodes[*i].vendor,
            Target::Unknown(_) => "generic",
        }
    }

    /// `Microarchitecture.family`: the unique root above the target; an unknown target is its
    /// own family.
    pub fn family(&self, t: &Target) -> Target {
        match t {
            Target::Known(i) => Target::Known(self.nodes[*i].family),
            Target::Unknown(_) => t.clone(),
        }
    }

    // The partial order of `Microarchitecture`: `a < b` iff a's name-closure is a proper
    // subset of b's, which within one graph is "a is an ancestor of b"; an unknown name is
    // incomparable to everything but itself, and equality is name equality.

    fn idx_lt(&self, a: usize, b: usize) -> bool {
        self.nodes[b].ancestor_set.contains(&a)
    }

    fn idx_le(&self, a: usize, b: usize) -> bool {
        a == b || self.idx_lt(a, b)
    }

    pub fn t_eq(&self, a: &Target, b: &Target) -> bool {
        match (a, b) {
            (Target::Known(i), Target::Known(j)) => i == j,
            (Target::Unknown(x), Target::Unknown(y)) => x == y,
            _ => false,
        }
    }

    pub fn t_lt(&self, a: &Target, b: &Target) -> bool {
        match (a, b) {
            (Target::Known(i), Target::Known(j)) => i != j && self.idx_lt(*i, *j),
            _ => false,
        }
    }

    pub fn t_le(&self, a: &Target, b: &Target) -> bool {
        self.t_eq(a, b) || self.t_lt(a, b)
    }

    pub fn t_gt(&self, a: &Target, b: &Target) -> bool {
        self.t_lt(b, a)
    }

    pub fn t_ge(&self, a: &Target, b: &Target) -> bool {
        self.t_eq(a, b) || self.t_gt(a, b)
    }

    /// `_closed_interval_targets` over node indices; the bounds are table members.
    fn closed_interval(&self, lower: Option<usize>, upper: usize) -> HashSet<usize> {
        let mut candidates: HashSet<usize> = self.nodes[upper].ancestor_set.clone();
        candidates.insert(upper);
        if let Some(lo) = lower {
            candidates.retain(|&t| self.idx_le(lo, t));
        }
        candidates
    }

    /// `_closed_interval_targets`: the targets in the closed interval between the bounds; a
    /// `None` lower bound means the family root.
    pub fn closed_interval_targets(
        &self,
        lower: Option<&Target>,
        upper: &Target,
    ) -> HashSet<Target> {
        let mut candidates: HashSet<Target> = match upper {
            Target::Known(u) => {
                let mut set: HashSet<Target> = self.nodes[*u]
                    .ancestor_set
                    .iter()
                    .map(|&i| Target::Known(i))
                    .collect();
                set.insert(upper.clone());
                set
            }
            Target::Unknown(_) => std::iter::once(upper.clone()).collect(),
        };
        if let Some(lo) = lower {
            candidates.retain(|t| self.t_ge(t, lo));
        }
        candidates
    }

    /// `_decompose_target_set`: a deterministic decomposition of a set of targets into
    /// interval elements; repeatedly emit the largest interval that starts at a minimal
    /// remaining target and stays inside the set.
    pub fn decompose_target_set(&self, targets: &HashSet<Target>) -> Vec<String> {
        let mut result = Vec::new();
        let mut remaining: HashSet<Target> = targets.clone();
        while !remaining.is_empty() {
            let m = remaining
                .iter()
                .filter(|t| !remaining.iter().any(|s| self.t_lt(s, t)))
                .min_by(|a, b| self.name_of(a).cmp(self.name_of(b)))
                .cloned()
                .unwrap();
            let mut best = m.clone();
            let mut members: HashSet<Target> = std::iter::once(m.clone()).collect();
            let mut sorted_remaining: Vec<&Target> = remaining.iter().collect();
            sorted_remaining.sort_by(|a, b| self.name_of(a).cmp(self.name_of(b)));
            for u in sorted_remaining {
                if !self.t_gt(u, &m) {
                    continue;
                }
                let candidates = self.closed_interval_targets(Some(&m), u);
                if candidates.len() > members.len() && candidates.is_subset(&remaining) {
                    best = u.clone();
                    members = candidates;
                }
            }
            if self.t_eq(&m, &best) {
                result.push(self.name_of(&m).to_string());
            } else if self.t_eq(&m, &self.family(&m)) {
                result.push(format!(":{}", self.name_of(&best)));
            } else {
                result.push(format!("{}:{}", self.name_of(&m), self.name_of(&best)));
            }
            for t in &members {
                remaining.remove(t);
            }
        }
        result
    }

    /// First normalization pass of `_canonical_target_range` on one element: `*` to `:`, a
    /// bounded interval whose lower bound is missing or the family root to `:hi` (or the
    /// singleton `hi` when `hi` is itself a root), and `x:x` to `x`.
    fn normalize_element(&self, raw: &str) -> String {
        let element = if raw == "*" { ":" } else { raw };
        let (t_min, t_sep, t_max) = partition(element);
        if t_sep && !t_max.is_empty() {
            let upper = self.make_target(t_max);
            let family = self.family(&upper);
            let family_name = self.name_of(&family);
            if t_min.is_empty() || t_min == family_name {
                return if t_max == family_name {
                    t_max.to_string()
                } else {
                    format!(":{t_max}")
                };
            } else if t_min == t_max {
                return t_min.to_string();
            }
        }
        element.to_string()
    }

    /// `_canonical_target_range`: the canonical string form of a target expression. Elements
    /// denoting no target are dropped (unless every element does), in-table elements are fused
    /// per family via [`Self::decompose_target_set`], open elements keep only the antichain of
    /// minimal lower bounds, and the result is the sorted comma-joined form.
    pub fn canonical_target_range(&self, name: &str) -> String {
        let elements: BTreeSet<String> =
            name.split(',').map(|e| self.normalize_element(e)).collect();

        if elements.len() <= 1 {
            return elements.into_iter().next().unwrap_or_default();
        }

        let mut opens: Vec<usize> = Vec::new(); // lower bounds of `x:` elements, all in table
        let mut union: HashSet<Target> = HashSet::new();
        let mut verbatim: BTreeSet<String> = BTreeSet::new(); // bounds outside the table
        for element in &elements {
            let (t_min, t_sep, t_max) = partition(element);
            if t_min.is_empty() && t_max.is_empty() {
                if t_sep {
                    // ':' is unbounded on both sides, so it contains every other element
                    return ":".to_string();
                }
                verbatim.insert(element.clone());
            } else if (!t_min.is_empty() && !self.by_name.contains_key(t_min))
                || (!t_max.is_empty() && !self.by_name.contains_key(t_max))
            {
                verbatim.insert(element.clone());
            } else if t_sep && t_max.is_empty() {
                opens.push(self.by_name[t_min]);
            } else {
                let lower = (!t_min.is_empty()).then(|| self.by_name[t_min]);
                let upper = self.by_name[if t_max.is_empty() { t_min } else { t_max }];
                union.extend(
                    self.closed_interval(lower, upper)
                        .into_iter()
                        .map(Target::Known),
                );
            }
        }

        if opens.is_empty() && union.is_empty() && verbatim.is_empty() {
            // every element denotes the empty set: keep them rather than returning nothing
            return elements.into_iter().collect::<Vec<_>>().join(",");
        }

        // keep the antichain of minimal lower bounds: `y:` contains `x:` whenever y is below x
        let opens: Vec<usize> = opens
            .iter()
            .copied()
            .filter(|&x| !opens.iter().any(|&y| y != x && self.idx_lt(y, x)))
            .collect();
        // an open element already denotes every target above its bound
        union.retain(|t| !opens.iter().any(|&x| self.t_ge(t, &Target::Known(x))));

        let mut kept = verbatim;
        kept.extend(opens.iter().map(|&x| format!("{}:", self.nodes[x].name)));
        kept.extend(self.decompose_target_set(&union));
        kept.into_iter().collect::<Vec<_>>().join(",")
    }

    /// `_satisfies_target_range`: whether every microarchitecture in the single element `rhs`
    /// is also in the single element `lhs`.
    pub fn satisfies_target_range(&self, lhs: &str, rhs: &str) -> bool {
        let (lhs_min, lhs_sep, lhs_max) = partition(lhs);
        let (rhs_min, rhs_sep, rhs_max) = partition(rhs);

        if !rhs_sep {
            // rhs is concrete: contained iff it falls within lhs's bounds. Comparing with
            // targets rather than names keeps bounds outside the table from failing.
            let t = self.make_target(rhs_min);
            if !lhs_sep {
                return rhs_min == lhs_min;
            }
            return (lhs_min.is_empty() || self.t_ge(&t, &self.make_target(lhs_min)))
                && (lhs_max.is_empty() || self.t_le(&t, &self.make_target(lhs_max)));
        }

        if !lhs_sep {
            // lhs is concrete: a range is inside it only by denoting that point too.
            return rhs_min == rhs_max && rhs_max == lhs_min;
        }

        // Both are ranges
        if !lhs_min.is_empty() {
            if rhs_min.is_empty() && rhs_max.is_empty() {
                return false;
            }
            let floor = if !rhs_min.is_empty() {
                self.make_target(rhs_min)
            } else {
                self.family(&self.make_target(rhs_max))
            };
            if !self.t_ge(&floor, &self.make_target(lhs_min)) {
                return false;
            }
        }
        if !lhs_max.is_empty()
            && (rhs_max.is_empty()
                || !self.t_le(&self.make_target(rhs_max), &self.make_target(lhs_max)))
        {
            return false;
        }
        true
    }

    /// `_covered_by_target_list`: whether every target the element denotes is in one of
    /// `rhs_elements`. A range with an in-table upper bound can be covered by several rhs
    /// elements jointly; an open element is only inside an open element.
    pub fn covered_by_target_list(&self, element: &str, rhs_elements: &[&str]) -> bool {
        if rhs_elements
            .iter()
            .any(|r| self.satisfies_target_range(r, element))
        {
            return true;
        }
        if rhs_elements.len() == 1 {
            return false;
        }
        let (t_min, _, t_max) = partition(element);
        if t_max.is_empty()
            || !self.by_name.contains_key(t_max)
            || (!t_min.is_empty() && !self.by_name.contains_key(t_min))
        {
            // a point is covered element-wise or not at all; so are unknown names
            return false;
        }
        let lower = (!t_min.is_empty()).then(|| self.by_name[t_min]);
        let members = self.closed_interval(lower, self.by_name[t_max]);
        members.iter().all(|&m| {
            rhs_elements
                .iter()
                .any(|r| self.satisfies_target_range(r, &self.nodes[m].name))
        })
    }

    /// `_parse_target_range`: (min, max) bounds of a single element, `None` meaning unbounded;
    /// for concrete elements min == max.
    pub fn parse_target_range(&self, element: &str) -> (Option<Target>, Option<Target>) {
        let (t_min, t_sep, t_max) = partition(element);
        if !t_sep {
            let t = self.make_target(t_min);
            return (Some(t.clone()), Some(t));
        }
        (
            (!t_min.is_empty()).then(|| self.make_target(t_min)),
            (!t_max.is_empty()).then(|| self.make_target(t_max)),
        )
    }

    /// `_minimal_upper_bounds`: the minimal targets above both `a` and `b`. The target graph
    /// is not a lattice, so incomparable bounds can have several minimal upper bounds; the
    /// result is in table order.
    pub fn minimal_upper_bounds(
        &self,
        a: Option<&Target>,
        b: Option<&Target>,
    ) -> Vec<Option<Target>> {
        let (a, b) = match (a, b) {
            (None, b) => return vec![b.cloned()],
            (a, None) => return vec![a.cloned()],
            (Some(a), Some(b)) => (a, b),
        };
        if self.name_of(a) == self.name_of(b) {
            return vec![Some(a.clone())];
        }
        if !self.t_eq(&self.family(a), &self.family(b)) {
            return vec![];
        }
        if self.t_ge(a, b) {
            return vec![Some(a.clone())];
        }
        if self.t_gt(b, a) {
            return vec![Some(b.clone())];
        }
        // both are incomparable table members of one family
        let (i, j) = match (a, b) {
            (Target::Known(i), Target::Known(j)) => (*i, *j),
            _ => unreachable!("unknown targets are incomparable and share no family"),
        };
        let key = (i.min(j), i.max(j));
        if let Some(cached) = self.mub_cache.lock().unwrap().get(&key) {
            return cached.iter().map(|&t| Some(Target::Known(t))).collect();
        }
        let family = self.nodes[i].family;
        let above: Vec<usize> = (0..self.nodes.len())
            .filter(|&t| self.nodes[t].family == family && self.idx_le(i, t) && self.idx_le(j, t))
            .collect();
        let minimal: Vec<usize> = above
            .iter()
            .copied()
            .filter(|&t| !above.iter().any(|&o| o != t && self.idx_lt(o, t)))
            .collect();
        let result = minimal.iter().map(|&t| Some(Target::Known(t))).collect();
        self.mub_cache.lock().unwrap().insert(key, minimal);
        result
    }

    /// `_maximal_lower_bounds`: the dual of [`Self::minimal_upper_bounds`]; the result is in
    /// table order (Python's is hash-seed dependent for multi-element results).
    pub fn maximal_lower_bounds(
        &self,
        a: Option<&Target>,
        b: Option<&Target>,
    ) -> Vec<Option<Target>> {
        let (a, b) = match (a, b) {
            (None, b) => return vec![b.cloned()],
            (a, None) => return vec![a.cloned()],
            (Some(a), Some(b)) => (a, b),
        };
        if self.name_of(a) == self.name_of(b) {
            return vec![Some(a.clone())];
        }
        if !self.t_eq(&self.family(a), &self.family(b)) {
            return vec![];
        }
        if self.t_le(a, b) {
            return vec![Some(a.clone())];
        }
        if self.t_lt(b, a) {
            return vec![Some(b.clone())];
        }
        let (i, j) = match (a, b) {
            (Target::Known(i), Target::Known(j)) => (*i, *j),
            _ => unreachable!("unknown targets are incomparable and share no family"),
        };
        let key = (i.min(j), i.max(j));
        if let Some(cached) = self.mlb_cache.lock().unwrap().get(&key) {
            return cached.iter().map(|&t| Some(Target::Known(t))).collect();
        }
        let below: Vec<usize> = (0..self.nodes.len())
            .filter(|t| {
                self.nodes[i].ancestor_set.contains(t) && self.nodes[j].ancestor_set.contains(t)
            })
            .collect();
        let maximal: Vec<usize> = below
            .iter()
            .copied()
            .filter(|&t| !below.iter().any(|&o| o != t && self.idx_lt(t, o)))
            .collect();
        let result = maximal.iter().map(|&t| Some(Target::Known(t))).collect();
        self.mlb_cache.lock().unwrap().insert(key, maximal);
        result
    }

    /// `ArchSpec._target_satisfies`: whether every microarchitecture in `lhs` is also in
    /// `rhs`; a missing rhs constrains nothing, a missing lhs satisfies only that.
    pub fn target_satisfies(&self, lhs: Option<&TargetExpr>, rhs: Option<&TargetExpr>) -> bool {
        let Some(rhs) = rhs else { return true };
        let Some(lhs) = lhs else { return false };
        let rhs_elements: Vec<&str> = rhs.as_str().split(',').collect();
        lhs.as_str()
            .split(',')
            .all(|l| self.covered_by_target_list(l, &rhs_elements))
    }

    /// `ArchSpec._target_intersects`: whether there is a microarchitecture in both targets.
    pub fn target_intersects(&self, lhs: Option<&TargetExpr>, rhs: Option<&TargetExpr>) -> bool {
        match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => !self.target_intersection(lhs, rhs).is_empty(),
            _ => true,
        }
    }

    /// `ArchSpec._target_intersection`: the elements of the target list denoting the targets
    /// both sides contain, in the same order as Python (up to the hash-seed dependence of
    /// multi-element maximal lower bounds, which here are in table order).
    pub fn target_intersection(&self, lhs: &TargetExpr, rhs: &TargetExpr) -> Vec<String> {
        let mut results: Vec<String> = Vec::new();

        for l_element in lhs.as_str().split(',') {
            for r_element in rhs.as_str().split(',') {
                // a concrete element intersects another element iff contained in it, and the
                // overlap is the concrete element itself
                if !r_element.contains(':') {
                    if self.satisfies_target_range(l_element, r_element) {
                        results.push(r_element.to_string());
                    }
                    continue;
                }
                if !l_element.contains(':') {
                    if self.satisfies_target_range(r_element, l_element) {
                        results.push(l_element.to_string());
                    }
                    continue;
                }
                let (l_min, l_max) = self.parse_target_range(l_element);
                let (r_min, r_max) = self.parse_target_range(r_element);
                let maxima = self.maximal_lower_bounds(l_max.as_ref(), r_max.as_ref());
                for n_min in self.minimal_upper_bounds(l_min.as_ref(), r_min.as_ref()) {
                    for n_max in &maxima {
                        match (&n_min, n_max) {
                            (None, None) => results.push(":".to_string()),
                            (None, Some(hi)) => results.push(format!(":{}", self.name_of(hi))),
                            (Some(lo), None) => results.push(format!("{}:", self.name_of(lo))),
                            (Some(lo), Some(hi)) => {
                                if self.name_of(lo) == self.name_of(hi) {
                                    results.push(self.name_of(lo).to_string());
                                } else if self.t_eq(&self.family(lo), &self.family(hi))
                                    && self.t_le(lo, hi)
                                {
                                    results.push(format!(
                                        "{}:{}",
                                        self.name_of(lo),
                                        self.name_of(hi)
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        results
    }

    /// `ArchSpec._target_constrain`: intersect `lhs` with `rhs`. Returns the new target and
    /// whether it changed; a missing rhs changes nothing, a missing lhs takes rhs, and
    /// disjoint targets are an error.
    pub fn target_constrain(
        &self,
        lhs: Option<&TargetExpr>,
        rhs: Option<&TargetExpr>,
    ) -> Result<(Option<TargetExpr>, bool), UnsatisfiableTarget> {
        let Some(rhs) = rhs else {
            return Ok((lhs.cloned(), false));
        };
        let Some(lhs) = lhs else {
            return Ok((Some(rhs.clone()), true));
        };
        let results = self.target_intersection(lhs, rhs);
        if results.is_empty() {
            return Err(UnsatisfiableTarget {
                lhs: lhs.as_str().to_string(),
                rhs: rhs.as_str().to_string(),
            });
        }
        // Targets are stored canonically, so the intersection equals lhs exactly when lhs is
        // already inside rhs.
        let intersection = TargetExpr::parse(self, &results.join(","));
        if intersection == *lhs {
            return Ok((Some(lhs.clone()), false));
        }
        Ok((Some(intersection), true))
    }
}
