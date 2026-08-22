// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The canonical comparison stream of `spack.spec.Spec`: the port of `_cmp_iter` and its
//! two nested generators. The reference lazily yields two callables — a node stream and a
//! canonical edge stream — where canonical edge ids follow breadth-first traversal order
//! (the WARNING-commented three-case block in `lib/spack/spack/spec.py`). The nested
//! callables here compute their sequence eagerly on call; the values, their order, and the
//! shared state between the two streams are identical to the reference generators.

use std::collections::{HashMap, HashSet, VecDeque};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};
use pyo3::IntoPyObjectExt;

use crate::algebra::{
    edge_child, edge_parent, has_deps, out_edges, ptr, spec_abstract_hash, spec_name,
};

/// `spack.traverse.sort_edges`: sort by child name first, then abstract hash, then full
/// edge comparison to break ties.
pub(crate) fn sort_edges<'py>(
    py: Python<'py>,
    edges: Vec<Bound<'py, PyAny>>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    if edges.len() <= 1 {
        return Ok(edges);
    }
    let keyed = PyList::empty(py);
    for edge in &edges {
        let child = edge_child(edge)?;
        let name = spec_name(&child)?;
        let hash = spec_abstract_hash(&child)?.unwrap_or_default();
        keyed.append((name, hash, edge))?;
    }
    keyed.call_method0("sort")?;
    keyed
        .iter()
        .map(|item| item.get_item(2))
        .collect::<PyResult<_>>()
}

/// The canonical edge tuple both streams yield:
/// `(parent_id, child_id, depflag, virtuals, direct, propagation, when)`.
fn edge_cmp_tuple<'py>(
    py: Python<'py>,
    edge: &Bound<'py, PyAny>,
    parent_id: usize,
    child_id: usize,
) -> PyResult<Bound<'py, PyTuple>> {
    let items: Vec<Py<PyAny>> = vec![
        parent_id.into_py_any(py)?,
        child_id.into_py_any(py)?,
        edge.getattr("depflag")?.unbind(),
        edge.getattr("virtuals")?.unbind(),
        edge.getattr("direct")?.unbind(),
        edge.getattr("propagation")?.unbind(),
        edge.getattr("when")?.unbind(),
    ];
    PyTuple::new(py, items)
}

/// The reference `node_ids` defaultdict: consistent ids in order of first access.
fn node_id(node_ids: &mut HashMap<usize, usize>, key: usize) -> usize {
    let next = node_ids.len();
    *node_ids.entry(key).or_insert(next)
}

/// State shared between the node and edge callables of one `_cmp_iter()` call, like the
/// reference nonlocals: the node stream computes it, the edge stream reads it.
#[pyclass(module = "spack_spec")]
pub struct CmpStream {
    spec: Py<PyAny>,
    sorted_l1_edges: Option<Py<PyList>>,
    edge_list: Option<Py<PyList>>,
}

/// The `nodes` callable of `_cmp_iter`.
#[pyclass(module = "spack_spec")]
pub struct CmpNodes {
    stream: Py<CmpStream>,
}

/// The `edges` callable of `_cmp_iter`.
#[pyclass(module = "spack_spec")]
pub struct CmpEdges {
    stream: Py<CmpStream>,
}

/// A BFS queue entry: an artificial root edge to a level-1 spec, or a real edge.
enum Item<'py> {
    Root(Bound<'py, PyAny>),
    Edge(Bound<'py, PyAny>),
}

#[pymethods]
impl CmpNodes {
    fn __call__(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let stream = self.stream.bind(py);
        let spec = { stream.borrow().spec.clone_ref(py) }.into_bound(py);
        let result = PyList::empty(py);

        // Level 0: root node — always yield the root (this node).
        result.append(spec.getattr("_cmp_node")?)?;
        if !has_deps(&spec)? {
            // Case 1: no dependencies, done.
            return Ok(result.unbind());
        }

        // Level 1: direct dependencies, in sorted order without tracking visited nodes.
        let mut deps_have_deps = false;
        let l1_edges = sort_edges(py, out_edges(&spec)?)?;
        {
            let mut state = stream.borrow_mut();
            state.sorted_l1_edges = Some(PyList::new(py, &l1_edges)?.unbind());
        }
        for edge in &l1_edges {
            let child = edge_child(edge)?;
            result.append(child.getattr("_cmp_node")?)?;
            if has_deps(&child)? {
                deps_have_deps = true;
            }
        }
        if !deps_have_deps {
            // Case 2: level 1 specs have no dependencies, done.
            return Ok(result.unbind());
        }

        // Case 3, level 2+: dependencies of direct dependencies. The reference calls
        // spack.traverse.traverse_edges(l1_specs, order="breadth", cover="edges",
        // root=False, visited={0}): a breadth-first walk over artificial root edges to the
        // l1 specs, expanding each node's sorted out-edges exactly once and yielding every
        // real edge once. The node_ids dict generates consistent ids in BFS order: the
        // root is 0, l1 starts at 1.
        let l1_specs = l1_edges
            .iter()
            .map(edge_child)
            .collect::<PyResult<Vec<_>>>()?;
        let mut node_ids: HashMap<usize, usize> = HashMap::new();
        node_id(&mut node_ids, ptr(&spec));
        for l1_spec in &l1_specs {
            node_id(&mut node_ids, ptr(l1_spec));
        }

        let edge_list = PyList::empty(py);
        stream.borrow_mut().edge_list = Some(edge_list.clone().unbind());

        // The reference passes visited={0}: a sentinel no id() ever equals.
        let mut expanded: HashSet<usize> = HashSet::new();
        expanded.insert(0);
        let mut queue: VecDeque<Item<'_>> = l1_specs.into_iter().map(Item::Root).collect();
        while let Some(item) = queue.pop_front() {
            let child = match &item {
                Item::Root(l1_spec) => l1_spec.clone(),
                Item::Edge(edge) => edge_child(edge)?,
            };
            if let Item::Edge(edge) = &item {
                // Yield each node only once, and generate a consistent id for it the first
                // time it's encountered.
                if !node_ids.contains_key(&ptr(&child)) {
                    result.append(child.getattr("_cmp_node")?)?;
                    node_id(&mut node_ids, ptr(&child));
                }

                let parent = edge_parent(edge)?;
                // Skip fake edge to root.
                if !parent.is_none() {
                    let parent_id = node_id(&mut node_ids, ptr(&parent));
                    let child_id = node_id(&mut node_ids, ptr(&child));
                    edge_list.append(edge_cmp_tuple(py, edge, parent_id, child_id)?)?;
                }
            }

            // cover="edges": drop dependencies of nodes expanded before.
            if expanded.insert(ptr(&child)) {
                for edge in sort_edges(py, out_edges(&child)?)? {
                    queue.push_back(Item::Edge(edge));
                }
            }
        }

        Ok(result.unbind())
    }
}

#[pymethods]
impl CmpEdges {
    fn __call__(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let stream = self.stream.bind(py);
        let (spec, sorted_l1_edges, edge_list) = {
            let state = stream.borrow();
            (
                state.spec.clone_ref(py).into_bound(py),
                state.sorted_l1_edges.as_ref().map(|l| l.clone_ref(py)),
                state.edge_list.as_ref().map(|l| l.clone_ref(py)),
            )
        };
        let result = PyList::empty(py);

        // No edges in a single-node graph.
        if !has_deps(&spec)? {
            return Ok(result.unbind());
        }

        // Level 1 edges all start with zero. Like the reference iterating the None
        // nonlocal, an edge stream consumed before the node stream fails with a TypeError.
        let sorted_l1_edges = sorted_l1_edges
            .ok_or_else(|| PyTypeError::new_err("'NoneType' object is not iterable"))?
            .into_bound(py);
        for (i, edge) in sorted_l1_edges.iter().enumerate() {
            result.append(edge_cmp_tuple(py, &edge, 0, i + 1)?)?;
        }

        // Yield remaining edges in the order they were encountered during traversal.
        if let Some(edge_list) = edge_list {
            let edge_list = edge_list.into_bound(py);
            if edge_list.len() > 0 {
                for item in edge_list.iter() {
                    result.append(item)?;
                }
            }
        }

        Ok(result.unbind())
    }
}

/// `Spec._cmp_iter()`: the `(nodes, edges)` pair of callables over a fresh shared state.
pub(crate) fn make_cmp_iter(slf: &Bound<'_, PyAny>) -> PyResult<Py<PyTuple>> {
    let py = slf.py();
    let stream = Py::new(
        py,
        CmpStream {
            spec: slf.clone().unbind(),
            sorted_l1_edges: None,
            edge_list: None,
        },
    )?;
    let nodes = Py::new(
        py,
        CmpNodes {
            stream: stream.clone_ref(py),
        },
    )?;
    let edges = Py::new(py, CmpEdges { stream })?;
    Ok(PyTuple::new(py, [nodes.into_py_any(py)?, edges.into_py_any(py)?])?.unbind())
}

/// Native `lazy_eq` over two Spec `_cmp_iter` streams, skipping the Python method lookup.
pub(crate) fn spec_lazy_eq(lhs: &Bound<'_, PyAny>, rhs: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = lhs.py();
    let lhs_iter = make_cmp_iter(lhs)?.into_bound(py);
    let rhs_iter = make_cmp_iter(rhs)?.into_bound(py);
    for i in 0..2 {
        if !crate::lazy::lazy_eq(&lhs_iter.get_item(i)?, &rhs_iter.get_item(i)?)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Native `lazy_lt` over two Spec `_cmp_iter` streams. Both streams always yield exactly
/// two nested sequences, so the generic value cases of `lazy_lt` cannot occur here.
pub(crate) fn spec_lazy_lt(lhs: &Bound<'_, PyAny>, rhs: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = lhs.py();
    let lhs_iter = make_cmp_iter(lhs)?.into_bound(py);
    let rhs_iter = make_cmp_iter(rhs)?.into_bound(py);
    for i in 0..2 {
        let left = lhs_iter.get_item(i)?;
        let right = rhs_iter.get_item(i)?;
        if crate::lazy::lazy_eq(&left, &right)? {
            continue;
        }
        return crate::lazy::lazy_lt(&left, &right);
    }
    Ok(false)
}
