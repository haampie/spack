// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The satisfies/intersects/constrain algebra over spec nodes and edges: the port of the
//! `_EdgeAlgebra` mixin, the module-level edge/condition helpers, and the algebra methods
//! of `spack.spec.Spec`, transcribed statement by statement from the reference
//! implementation in `lib/spack/spack/spec.py` (which remains the python-mode
//! implementation). Graph state lives in Python containers, so this module is plumbing
//! over `Bound<PyAny>` handles; borrows of our pyclasses are kept short and never held
//! across recursion into another node.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CStr;

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::edge::DependencySpec;
use crate::registry;
use crate::spec::Spec;

/// `spack.deptypes` bit values, transcribed from `lib/spack/spack/deptypes.py`.
pub(crate) const DT_LINK: u64 = 0b0001;
pub(crate) const DT_RUN: u64 = 0b0010;
pub(crate) const DT_BUILD: u64 = 0b0100;
pub(crate) const DT_TEST: u64 = 0b1000;
pub(crate) const DT_ALL: u64 = DT_BUILD | DT_LINK | DT_RUN | DT_TEST;

/// `spack.enums.PropagationPolicy.NONE` / `.PREFERENCE`.
pub(crate) const PROPAGATION_NONE: i64 = 1;
pub(crate) const PROPAGATION_PREFERENCE: i64 = 2;

// --------------------------------------------------------------------------------------
// Registry access and error construction
// --------------------------------------------------------------------------------------

pub(crate) fn registered<'py>(
    py: Python<'py>,
    cell: &'static PyOnceLock<Py<PyAny>>,
    what: &str,
) -> PyResult<Bound<'py, PyAny>> {
    match cell.get(py) {
        Some(obj) => Ok(obj.bind(py).clone()),
        None => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "spack_spec: {what} is not registered"
        ))),
    }
}

/// Whether an exception raised from a nested call is a `spack.error.SpecError`.
pub(crate) fn is_spec_error(py: Python<'_>, err: &PyErr) -> PyResult<bool> {
    let cls = registered(py, &registry::SPEC_ERROR, "SpecError")?;
    Ok(err.matches(py, &cls)?)
}

// --------------------------------------------------------------------------------------
// Recursion guard: deep or cyclic graphs raise RecursionError as in Python, instead of
// exhausting the native stack.
// --------------------------------------------------------------------------------------

pub(crate) struct RecursionGuard;

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        unsafe { pyo3::ffi::Py_LeaveRecursiveCall() }
    }
}

pub(crate) fn enter_recursion(py: Python<'_>, what: &'static CStr) -> PyResult<RecursionGuard> {
    if unsafe { pyo3::ffi::Py_EnterRecursiveCall(what.as_ptr()) } != 0 {
        Err(PyErr::take(py).unwrap_or_else(|| {
            pyo3::exceptions::PyRecursionError::new_err("maximum recursion depth exceeded")
        }))
    } else {
        Ok(RecursionGuard)
    }
}

// --------------------------------------------------------------------------------------
// Short-borrow accessors for the Rust Spec and DependencySpec state
// --------------------------------------------------------------------------------------

pub(crate) fn is_empty_spec(obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = obj.py();
    Ok(registered(py, &registry::EMPTY_SPEC, "EMPTY_SPEC")?.is(obj))
}

pub(crate) fn empty_spec_obj(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    registered(py, &registry::EMPTY_SPEC, "EMPTY_SPEC")
}

pub(crate) fn spec_name(s: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(s.downcast::<Spec>()?.borrow().name.clone())
}

pub(crate) fn spec_namespace(s: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    Ok(s.downcast::<Spec>()?.borrow().namespace.clone())
}

pub(crate) fn spec_abstract_hash(s: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    Ok(s.downcast::<Spec>()?.borrow().abstract_hash.clone())
}

/// `None` and the empty string are both falsy, as in the reference truthiness checks.
pub(crate) fn truthy_str(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|v| !v.is_empty())
}

pub(crate) fn is_concrete(s: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(s.downcast::<Spec>()?.borrow()._concrete)
}

/// A stored component handle, or the Python `None` object when unset, so downstream
/// method calls fail with the same `AttributeError` the reference would produce.
pub(crate) fn handle_or_none<'py>(
    py: Python<'py>,
    handle: &Option<Py<PyAny>>,
) -> Bound<'py, PyAny> {
    match handle {
        Some(h) => h.bind(py).clone(),
        None => py.None().into_bound(py),
    }
}

pub(crate) fn spec_versions<'py>(s: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r.versions.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle_or_none(s.py(), &handle))
}

pub(crate) fn spec_variants<'py>(s: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r.variants.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle_or_none(s.py(), &handle))
}

pub(crate) fn spec_compiler_flags<'py>(s: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r.compiler_flags.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle_or_none(s.py(), &handle))
}

/// The stored architecture, mapped to `None` when unset or the Python `None` object, so
/// callers mirror the reference `is not None` checks.
pub(crate) fn spec_architecture<'py>(s: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r.architecture.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle
        .map(|h| h.into_bound(s.py()))
        .filter(|h| !h.is_none()))
}

fn spec_provided_virtuals<'py>(s: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r._provided_virtuals.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle
        .map(|h| h.into_bound(s.py()))
        .filter(|h| !h.is_none()))
}

pub(crate) fn spec_dependencies_map<'py>(
    s: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r._dependencies.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle
        .map(|h| h.into_bound(s.py()))
        .filter(|h| !h.is_none()))
}

pub(crate) fn spec_dependents_map<'py>(
    s: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let cell = s.downcast::<Spec>()?;
    let handle = {
        let r = cell.borrow();
        r._dependents.as_ref().map(|h| h.clone_ref(s.py()))
    };
    Ok(handle
        .map(|h| h.into_bound(s.py()))
        .filter(|h| !h.is_none()))
}

/// `bool(spec._dependencies)`: an unset or empty edge map is falsy.
pub(crate) fn has_deps(s: &Bound<'_, PyAny>) -> PyResult<bool> {
    match spec_dependencies_map(s)? {
        Some(map) => map.is_truthy(),
        None => Ok(false),
    }
}

/// A snapshot of the outgoing edges, flattened in edge-map order: what the reference
/// `edges_to_dependencies()` returns when called without filters.
pub(crate) fn out_edges<'py>(s: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    edges_of(&spec_dependencies_map(s)?)
}

/// A snapshot of the incoming edges: `edges_from_dependents()` without filters.
pub(crate) fn in_edges<'py>(s: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    edges_of(&spec_dependents_map(s)?)
}

fn edges_of<'py>(map: &Option<Bound<'py, PyAny>>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut result = Vec::new();
    if let Some(map) = map {
        for bucket in map.downcast::<PyDict>()?.values() {
            for edge in bucket.downcast::<PyList>()? {
                result.push(edge);
            }
        }
    }
    Ok(result)
}

pub(crate) fn edge_parent<'py>(e: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = e.downcast::<DependencySpec>()?;
    let handle = {
        let r = cell.borrow();
        r.parent.as_ref().map(|h| h.clone_ref(e.py()))
    };
    Ok(handle_or_none(e.py(), &handle))
}

pub(crate) fn edge_child<'py>(e: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = e.downcast::<DependencySpec>()?;
    let handle = {
        let r = cell.borrow();
        r.spec.as_ref().map(|h| h.clone_ref(e.py()))
    };
    Ok(handle_or_none(e.py(), &handle))
}

pub(crate) fn edge_when<'py>(e: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cell = e.downcast::<DependencySpec>()?;
    let handle = {
        let r = cell.borrow();
        r.when.as_ref().map(|h| h.clone_ref(e.py()))
    };
    Ok(handle_or_none(e.py(), &handle))
}

pub(crate) fn edge_depflag(e: &Bound<'_, PyAny>) -> PyResult<u64> {
    Ok(e.downcast::<DependencySpec>()?.borrow().depflag)
}

pub(crate) fn edge_virtuals(e: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    Ok(e.downcast::<DependencySpec>()?.borrow().virtuals.clone())
}

pub(crate) fn edge_direct(e: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(e.downcast::<DependencySpec>()?.borrow().direct)
}

pub(crate) fn edge_propagation(e: &Bound<'_, PyAny>) -> PyResult<i64> {
    Ok(e.downcast::<DependencySpec>()?.borrow().propagation)
}

pub(crate) fn ptr(obj: &Bound<'_, PyAny>) -> usize {
    obj.as_ptr() as usize
}

// --------------------------------------------------------------------------------------
// Module-level helpers: `_merged_when`, `_rename_node`, `_add_edge_to_map`
// --------------------------------------------------------------------------------------

/// `_merged_when`: the condition under which an edge merged from two edges applies.
fn merged_when<'py>(
    lhs: &Bound<'py, PyAny>,
    rhs: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    if spec_satisfies(lhs, rhs, true)? {
        return Ok(rhs.clone());
    }
    if spec_satisfies(rhs, lhs, true)? {
        return Ok(lhs.clone());
    }
    let merged = lhs.call_method0("copy")?;
    merged.call_method1("constrain", (rhs,))?;
    Ok(merged)
}

/// `_add_edge_to_map`: append the edge to the bucket for `key`, keeping it sorted.
pub(crate) fn add_edge_to_map(
    edge_map: &Bound<'_, PyAny>,
    key: &str,
    edge: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if edge_map.contains(key)? {
        let lst = edge_map.get_item(key)?;
        lst.call_method1("append", (edge,))?;
        lst.call_method0("sort")?;
    } else {
        let lst = PyList::new(edge_map.py(), [edge])?;
        edge_map.set_item(key, lst)?;
    }
    Ok(())
}

/// The bucket entries that are not `edge` itself, as a new list.
pub(crate) fn siblings_of<'py>(
    bucket: &Bound<'py, PyAny>,
    edge: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(bucket.py());
    for item in bucket.try_iter()? {
        let item = item?;
        if !item.is(edge) {
            result.append(item)?;
        }
    }
    Ok(result)
}

/// `_rename_node`: rename a node, rekeying the dependency map of each of its dependents.
fn rename_node(spec: &Bound<'_, PyAny>, name: &str) -> PyResult<()> {
    let incoming = in_edges(spec)?;
    let old_name = spec_name(spec)?;
    for edge in &incoming {
        let deps_map = edge_parent(edge)?.getattr("_dependencies")?;
        let bucket = deps_map.get_item(&old_name)?;
        let siblings = siblings_of(&bucket, edge)?;
        if siblings.len() > 0 {
            deps_map.set_item(&old_name, siblings)?;
        } else {
            deps_map.del_item(&old_name)?;
        }
    }

    spec.setattr("name", name)?;

    for edge in &incoming {
        let deps_map = edge_parent(edge)?.getattr("_dependencies")?;
        add_edge_to_map(&deps_map, name, edge)?;
    }
    Ok(())
}

// --------------------------------------------------------------------------------------
// Module-level condition and edge predicates
// --------------------------------------------------------------------------------------

/// `constrains_only_name_and_versions`.
fn constrains_only_name_and_versions(spec: &Bound<'_, PyAny>) -> PyResult<bool> {
    let variants = spec_variants(spec)?;
    if !variants.is_none() && variants.is_truthy()? {
        return Ok(false);
    }
    let flags = spec_compiler_flags(spec)?;
    if !flags.is_none() && flags.is_truthy()? {
        return Ok(false);
    }
    Ok(spec_architecture(spec)?.is_none()
        && spec_namespace(spec)?.is_none()
        && !truthy_str(&spec_abstract_hash(spec)?)
        && !has_deps(spec)?)
}

/// `_condition_must_hold`: whether the node alone guarantees an edge's when condition.
pub(crate) fn condition_must_hold(
    when: &Bound<'_, PyAny>,
    node: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    if is_empty_spec(when)? {
        return Ok(true);
    }
    Ok(!has_deps(when)? && spec_satisfies(node, when, false)?)
}

/// `_must_hold_predicate`: the merged node is built on first use.
struct MustHold<'py> {
    lhs: Bound<'py, PyAny>,
    rhs: Bound<'py, PyAny>,
    merged_node: Option<Bound<'py, PyAny>>,
}

impl<'py> MustHold<'py> {
    fn new(lhs: &Bound<'py, PyAny>, rhs: &Bound<'py, PyAny>) -> Self {
        MustHold {
            lhs: lhs.clone(),
            rhs: rhs.clone(),
            merged_node: None,
        }
    }

    fn must_hold(&mut self, when: &Bound<'py, PyAny>) -> PyResult<bool> {
        if is_empty_spec(when)? {
            return Ok(true);
        }
        if self.merged_node.is_none() {
            let py = self.lhs.py();
            let kwargs = PyDict::new(py);
            kwargs.set_item("deps", false)?;
            let merged = self.lhs.call_method("copy", (), Some(&kwargs))?;
            spec_merge(&merged, &self.rhs, false)?;
            self.merged_node = Some(merged);
        }
        condition_must_hold(when, self.merged_node.as_ref().unwrap())
    }
}

/// `_satisfies_edge_attributes`: edge attributes and the target node, not the parent.
pub(crate) fn satisfies_edge_attributes(
    lhs: &Bound<'_, PyAny>,
    rhs: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let py = lhs.py();
    let lhs_child = edge_child(lhs)?;
    let rhs_child = edge_child(rhs)?;
    let lhs_name = spec_name(&lhs_child)?;
    let rhs_name = spec_name(&rhs_child)?;
    let lhs_virtuals = edge_virtuals(lhs)?;

    let name_mismatch = !rhs_name.is_empty() && lhs_name != rhs_name;
    if name_mismatch && !lhs_virtuals.contains(&rhs_name) {
        return Ok(false);
    }

    if !spec_satisfies(&edge_when(rhs)?, &edge_when(lhs)?, true)? {
        return Ok(false);
    }

    // Subset semantics for virtuals
    for v in edge_virtuals(rhs)? {
        if !lhs_virtuals.contains(&v) {
            return Ok(false);
        }
    }

    // Subset semantics for dependency types
    let rhs_depflag = edge_depflag(rhs)?;
    if (edge_depflag(lhs)? & rhs_depflag) != rhs_depflag {
        return Ok(false);
    }

    if !name_mismatch {
        return satisfies_node(&lhs_child, &rhs_child);
    }

    // Right-hand side is a virtual provided by the left-hand side: only names and
    // versions are supported on virtuals.
    if !constrains_only_name_and_versions(&rhs_child)? {
        return Ok(false);
    }

    // Frozen virtual versions are only consulted when rhs narrows them.
    let any_version = registered(py, &registry::ANY_VERSION, "any_version")?;
    if spec_versions(&rhs_child)?.ne(&any_version)? && !provides_virtual(&lhs_child, &rhs_child)? {
        return Ok(false);
    }

    Ok(true)
}

/// `_satisfying_edges`, collected eagerly: the checks are pure, so the set is identical
/// to what the reference generator yields.
fn satisfying_edges<'py>(
    lhs_node: &Bound<'py, PyAny>,
    rhs_edge: &Bound<'py, PyAny>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let py = lhs_node.py();
    let mut result = Vec::new();

    // First check direct deps of all types; only abstract specs require the direct flag.
    let require_direct = edge_direct(rhs_edge)? && !is_concrete(lhs_node)?;
    for lhs_edge in out_edges(lhs_node)? {
        if require_direct && !edge_direct(&lhs_edge)? {
            continue;
        }
        if satisfies_edge_attributes(&lhs_edge, rhs_edge)? {
            result.push(lhs_edge);
        }
    }

    // Include the historical compiler node if available as an ad-hoc edge.
    let compiler_spec = lhs_node
        .getattr("annotations")?
        .getattr("compiler_node_attribute")?;
    if !compiler_spec.is_none() {
        let cls = registered(py, &registry::DEPENDENCY_SPEC_CLASS, "DependencySpec class")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("depflag", DT_BUILD)?;
        kwargs.set_item("virtuals", ("c", "cxx", "fortran"))?;
        kwargs.set_item("direct", true)?;
        let compiler_edge = cls.call((lhs_node, &compiler_spec), Some(&kwargs))?;
        if satisfies_edge_attributes(&compiler_edge, rhs_edge)? {
            result.push(compiler_edge);
        }
    }

    if edge_direct(rhs_edge)? {
        return Ok(result);
    }

    // BFS through link/run transitive deps (skip depth 1, already checked). Nodes with
    // multiple in-edges are expanded only once, but every in-edge is a candidate.
    let depflag = DT_LINK | DT_RUN;
    let mut queue: VecDeque<Bound<'py, PyAny>> = VecDeque::new();
    for e in out_edges(lhs_node)? {
        if edge_depflag(&e)? & depflag != 0 {
            queue.push_back(e);
        }
    }
    let mut expanded: HashSet<usize> = HashSet::new();
    expanded.insert(ptr(lhs_node));
    while let Some(lhs_edge) = queue.pop_front() {
        // depth 1 was collected by the loop over direct edges above
        if !edge_parent(&lhs_edge)?.is(lhs_node) && satisfies_edge_attributes(&lhs_edge, rhs_edge)?
        {
            result.push(lhs_edge.clone());
        }

        let child = edge_child(&lhs_edge)?;
        if expanded.insert(ptr(&child)) && has_deps(&child)? {
            for e in out_edges(&child)? {
                if edge_depflag(&e)? & depflag != 0 {
                    queue.push_back(e);
                }
            }
        }
    }
    Ok(result)
}

/// `_satisfies_dependencies`: every dependency edge of `rhs` is satisfied by some edge of
/// `lhs`.
fn satisfies_dependencies(lhs: &Bound<'_, PyAny>, rhs: &Bound<'_, PyAny>) -> PyResult<bool> {
    let _guard = enter_recursion(lhs.py(), c" in _satisfies_dependencies")?;
    for rhs_edge in out_edges(rhs)? {
        // Skip rhs edges whose when condition doesn't apply to the lhs node.
        let when = edge_when(&rhs_edge)?;
        if !is_empty_spec(&when)? && !condition_can_hold(lhs, &when)? {
            continue;
        }
        let edges = satisfying_edges(lhs, &rhs_edge)?;
        let rhs_child = edge_child(&rhs_edge)?;
        if is_concrete(&rhs_child)? || !has_deps(&rhs_child)? {
            if edges.is_empty() {
                return Ok(false);
            }
        } else {
            let mut any = false;
            for e in &edges {
                if satisfies_dependencies(&edge_child(e)?, &rhs_child)? {
                    any = true;
                    break;
                }
            }
            if !any {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// --------------------------------------------------------------------------------------
// Spec: satisfies and its node-level pieces
// --------------------------------------------------------------------------------------

/// `Spec._autospec`: pass a Spec through, parse anything else through the registered
/// Python Spec class.
pub fn autospec<'py>(obj: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let cls = registered(obj.py(), &registry::SPEC_CLASS, "the Spec class")?;
    if obj.is_instance(&cls)? {
        Ok(obj.clone())
    } else {
        cls.call1((obj,))
    }
}

/// `Spec.satisfies`.
pub fn spec_satisfies(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    deps: bool,
) -> PyResult<bool> {
    let _guard = enter_recursion(slf.py(), c" in Spec.satisfies")?;
    if is_empty_spec(other)? {
        return Ok(true);
    }

    let other = autospec(other)?;

    if !satisfies_node(slf, &other)? {
        return Ok(false);
    }

    // If there are no dependencies on the rhs, or we don't recurse, they are satisfied.
    if !deps || !has_deps(&other)? {
        return Ok(true);
    }

    satisfies_dependencies(slf, &other)
}

/// `Spec._provides_virtual`: consults the provided virtual versions frozen on the node.
pub fn provides_virtual(slf: &Bound<'_, PyAny>, virtual_spec: &Bound<'_, PyAny>) -> PyResult<bool> {
    let provided = spec_provided_virtuals(slf)?;
    let virtual_name = spec_name(virtual_spec)?;
    let provided = match provided {
        Some(p) if !virtual_name.is_empty() => p,
        _ => return Ok(false),
    };
    let versions = provided.call_method1("get", (virtual_name,))?;
    if versions.is_none() {
        return Ok(false);
    }
    versions
        .call_method1("intersects", (spec_versions(virtual_spec)?,))?
        .extract()
}

/// `Spec._satisfies_node`: compares self and other without looking at dependencies.
pub fn satisfies_node(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    if is_concrete(other)? {
        // The left-hand side must be the same singleton with identical hash.
        if !is_concrete(slf)? {
            return Ok(false);
        }
        let self_hash: String = slf.call_method0("dag_hash")?.extract()?;
        let other_hash: String = other.call_method0("dag_hash")?.extract()?;
        return Ok(self_hash == other_hash);
    }

    let self_name = spec_name(slf)?;
    let other_name = spec_name(other)?;
    if !other_name.is_empty() && self_name.is_empty() {
        return Ok(false);
    }

    if self_name != other_name && !self_name.is_empty() && !other_name.is_empty() {
        // Name mismatch can still be satisfiable if lhs provides the virtual mentioned by
        // rhs; other.versions refers to the virtual's versions, not the provider's.
        return provides_virtual(slf, other);
    }

    // If the right-hand side has an abstract hash, make sure it's a prefix of the
    // left-hand side's (abstract) hash.
    let other_hash = spec_abstract_hash(other)?;
    if truthy_str(&other_hash) {
        let other_hash = other_hash.unwrap();
        let compare_hash: Option<String> = if is_concrete(slf)? {
            Some(slf.call_method0("dag_hash")?.extract()?)
        } else {
            spec_abstract_hash(slf)?
        };
        if !compare_hash
            .as_deref()
            .is_some_and(|h| !h.is_empty() && h.starts_with(&other_hash))
        {
            return Ok(false);
        }
    }

    let other_namespace = spec_namespace(other)?;
    if other_namespace.is_some() && spec_namespace(slf)? != other_namespace {
        return Ok(false);
    }

    if !spec_versions(slf)?
        .call_method1("satisfies", (spec_versions(other)?,))?
        .extract::<bool>()?
    {
        return Ok(false);
    }

    if !satisfies_variants(slf, other)? {
        return Ok(false);
    }

    let self_arch = spec_architecture(slf)?;
    let other_arch = spec_architecture(other)?;
    let self_arch_truthy = match &self_arch {
        Some(a) => a.is_truthy()?,
        None => false,
    };
    let other_arch_truthy = match &other_arch {
        Some(a) => a.is_truthy()?,
        None => false,
    };
    if self_arch_truthy && other_arch_truthy {
        if !self_arch
            .unwrap()
            .call_method1("satisfies", (other_arch.unwrap(),))?
            .extract::<bool>()?
        {
            return Ok(false);
        }
    } else if other_arch_truthy && !self_arch_truthy {
        return Ok(false);
    }

    if !spec_compiler_flags(slf)?
        .call_method1("satisfies", (spec_compiler_flags(other)?,))?
        .extract::<bool>()?
    {
        return Ok(false);
    }

    Ok(true)
}

/// `Spec._satisfies_variants`.
pub fn satisfies_variants(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    if is_concrete(slf)? {
        satisfies_variants_when_self_concrete(slf, other)
    } else {
        satisfies_variants_when_self_abstract(slf, other)
    }
}

/// The unique nodes of `self.traverse()`: any-deptype reachability from the root, each
/// node once by identity. The reference conjunctions read only the node set, so the
/// pre-order/breadth distinction is immaterial.
fn traverse_nodes<'py>(root: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut visited: HashSet<usize> = HashSet::new();
    visited.insert(ptr(root));
    let mut result = Vec::new();
    let mut queue: VecDeque<Bound<'py, PyAny>> = VecDeque::new();
    queue.push_back(root.clone());
    while let Some(node) = queue.pop_front() {
        for edge in out_edges(&node)? {
            let child = edge_child(&edge)?;
            if visited.insert(ptr(&child)) {
                queue.push_back(child);
            }
        }
        result.push(node);
    }
    Ok(result)
}

fn partition_variants(variants: &Bound<'_, PyAny>) -> PyResult<(Vec<String>, Vec<String>)> {
    variants.call_method0("partition_variants")?.extract()
}

/// `variants[name].satisfies(other_variants[name])`.
fn variant_satisfies(
    variants: &Bound<'_, PyAny>,
    other_variants: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<bool> {
    variants
        .get_item(name)?
        .call_method1("satisfies", (other_variants.get_item(name)?,))?
        .extract()
}

/// `Spec._satisfies_variants_when_self_concrete`.
pub fn satisfies_variants_when_self_concrete(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let other_variants = spec_variants(other)?;
    let (non_propagating, propagating) = partition_variants(&other_variants)?;
    let self_variants = spec_variants(slf)?;
    let mut result = true;
    for name in &non_propagating {
        if !(self_variants.contains(name)?
            && variant_satisfies(&self_variants, &other_variants, name)?)
        {
            result = false;
            break;
        }
    }
    if propagating.is_empty() {
        return Ok(result);
    }

    for node in traverse_nodes(slf)? {
        let node_variants = spec_variants(&node)?;
        for name in &propagating {
            if node_variants.contains(name)?
                && !variant_satisfies(&node_variants, &other_variants, name)?
            {
                return Ok(false);
            }
        }
    }
    Ok(result)
}

/// `Spec._satisfies_variants_when_self_abstract`.
pub fn satisfies_variants_when_self_abstract(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let other_variants = spec_variants(other)?;
    let self_variants = spec_variants(slf)?;
    let (other_non_propagating, other_propagating) = partition_variants(&other_variants)?;
    let (self_non_propagating, self_propagating) = partition_variants(&self_variants)?;

    // First check variants without propagation set
    let mut result = true;
    for name in &other_non_propagating {
        let ok = self_non_propagating.contains(name) && {
            let self_variant = self_variants.get_item(name)?;
            self_variant.getattr("propagate")?.is_truthy()?
                || variant_satisfies(&self_variants, &other_variants, name)?
        };
        if !ok {
            result = false;
            break;
        }
    }
    if !result || (other_propagating.is_empty() && self_propagating.is_empty()) {
        return Ok(result);
    }

    // Check that self doesn't contradict variants propagated by other
    if !other_propagating.is_empty() {
        for node in traverse_nodes(slf)? {
            let node_variants = spec_variants(&node)?;
            for name in &other_propagating {
                if node_variants.contains(name)?
                    && !variant_satisfies(&node_variants, &other_variants, name)?
                {
                    return Ok(false);
                }
            }
        }
    }

    // Check that other doesn't contradict variants propagated by self
    if !self_propagating.is_empty() {
        for node in traverse_nodes(other)? {
            let node_variants = spec_variants(&node)?;
            for name in &self_propagating {
                if node_variants.contains(name)?
                    && !variant_satisfies(&node_variants, &self_variants, name)?
                {
                    return Ok(false);
                }
            }
        }
    }

    Ok(result)
}

// --------------------------------------------------------------------------------------
// Spec: disjointness and conflict reasons
// --------------------------------------------------------------------------------------

/// `Spec._disjoint_reason`.
pub fn disjoint_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    deps: bool,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    let _guard = enter_recursion(py, c" in Spec._disjoint_reason")?;
    if is_empty_spec(other)? {
        return Ok(None);
    }

    // A concrete spec is a singleton, so intersection reduces to satisfaction; a concrete
    // side is compared whole.
    if is_concrete(slf)? || is_concrete(other)? {
        let (lhs, rhs) = if is_concrete(slf)? {
            (slf, other)
        } else {
            (other, slf)
        };
        if spec_satisfies(lhs, rhs, true)? {
            return Ok(None);
        }
        let error = registered(
            py,
            &registry::UNSATISFIABLE_SPEC_ERROR,
            "UnsatisfiableSpecError",
        )?
        .call1((slf, other, "constrain a concrete spec"))?;
        return Ok(Some(error.unbind()));
    }

    let error = disjoint_node_reason(slf, other)?;
    if error.is_some() || !deps {
        return Ok(error);
    }

    disjoint_dependencies_reason(slf, other)
}

/// `Spec._conflict_reason`.
pub fn conflict_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    let _guard = enter_recursion(py, c" in Spec._conflict_reason")?;
    if is_empty_spec(other)? {
        return Ok(None);
    }

    // A concrete spec is a singleton with no internal pairs, so both questions reduce to
    // satisfaction there.
    if is_concrete(slf)? || is_concrete(other)? {
        let (lhs, rhs) = if is_concrete(slf)? {
            (slf, other)
        } else {
            (other, slf)
        };
        if spec_satisfies(lhs, rhs, true)? {
            return Ok(None);
        }
        let error = registered(
            py,
            &registry::UNSATISFIABLE_SPEC_ERROR,
            "UnsatisfiableSpecError",
        )?
        .call1((slf, other, "constrain a concrete spec"))?;
        return Ok(Some(error.unbind()));
    }

    if let Some(error) = disjoint_node_reason(slf, other)? {
        return Ok(Some(error));
    }

    conflicting_dependencies_reason(slf, other)
}

/// `Spec._disjoint_node_reason`.
pub fn disjoint_node_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    // Two abstract nodes with different names do not intersect.
    let self_name = spec_name(slf)?;
    let other_name = spec_name(other)?;
    if self_name != other_name && !self_name.is_empty() && !other_name.is_empty() {
        let error = registered(
            py,
            &registry::UNSAT_SPEC_NAME_ERROR,
            "UnsatisfiableSpecNameError",
        )?
        .call1((self_name, other_name))?;
        return Ok(Some(error.unbind()));
    }

    disjoint_node_content_reason(slf, other)
}

/// `Spec._disjoint_node_content_reason`.
pub fn disjoint_node_content_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    let self_versions = spec_versions(slf)?;
    let other_versions = spec_versions(other)?;
    if !self_versions
        .call_method1("intersects", (&other_versions,))?
        .extract::<bool>()?
    {
        let error = registered(
            py,
            &registry::UNSAT_VERSION_SPEC_ERROR,
            "UnsatisfiableVersionSpecError",
        )?
        .call1((self_versions, other_versions))?;
        return Ok(Some(error.unbind()));
    }

    disjoint_node_attributes_reason(slf, other)
}

/// `Spec._disjoint_node_attributes_reason`.
pub fn disjoint_node_attributes_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    let self_hash = spec_abstract_hash(slf)?;
    let other_hash = spec_abstract_hash(other)?;
    if truthy_str(&self_hash) && truthy_str(&other_hash) {
        let (sh, oh) = (
            self_hash.as_deref().unwrap(),
            other_hash.as_deref().unwrap(),
        );
        if !sh.starts_with(oh) && !oh.starts_with(sh) {
            let error = registered(py, &registry::INVALID_HASH_ERROR, "InvalidHashError")?
                .call1((slf, oh))?;
            return Ok(Some(error.unbind()));
        }
    }

    let self_namespace = spec_namespace(slf)?;
    let other_namespace = spec_namespace(other)?;
    if self_namespace.is_some() && other_namespace.is_some() && self_namespace != other_namespace {
        let error = registered(
            py,
            &registry::UNSAT_SPEC_NAME_ERROR,
            "UnsatisfiableSpecNameError",
        )?
        .call1((slf.getattr("fullname")?, other.getattr("fullname")?))?;
        return Ok(Some(error.unbind()));
    }

    let self_variants = spec_variants(slf)?.getattr("dict")?;
    let other_variants = spec_variants(other)?.getattr("dict")?;
    let items: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> =
        other_variants.downcast::<PyDict>()?.iter().collect();
    for (name, other_variant) in items {
        let self_variant = self_variants.call_method1("get", (name,))?;
        if !self_variant.is_none()
            && !self_variant
                .call_method1("intersects", (&other_variant,))?
                .extract::<bool>()?
        {
            let error = registered(
                py,
                &registry::UNSATISFIABLE_VARIANT_ERROR,
                "UnsatisfiableVariantSpecError",
            )?
            .call1((self_variant, other_variant))?;
            return Ok(Some(error.unbind()));
        }
    }

    let self_arch = spec_architecture(slf)?;
    let other_arch = spec_architecture(other)?;
    if let (Some(self_arch), Some(other_arch)) = (self_arch, other_arch) {
        if !self_arch
            .call_method1("intersects", (&other_arch,))?
            .extract::<bool>()?
        {
            let error = registered(
                py,
                &registry::UNSAT_ARCH_ERROR,
                "UnsatisfiableArchitectureSpecError",
            )?
            .call1((self_arch, other_arch))?;
            return Ok(Some(error.unbind()));
        }
    }

    // Compiler flags always intersect, since merging them is a union.
    Ok(None)
}

fn unsat_dependency_error(
    py: Python<'_>,
    provided: &Bound<'_, PyAny>,
    required: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let error = registered(
        py,
        &registry::UNSAT_DEPENDENCY_SPEC_ERROR,
        "UnsatisfiableDependencySpecError",
    )?
    .call1((provided, required))?;
    Ok(error.unbind())
}

/// `Spec._disjoint_dependencies_reason`.
pub fn disjoint_dependencies_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    if !has_deps(slf)? && !has_deps(other)? {
        return Ok(None);
    }

    let mut must_hold = MustHold::new(slf, other);

    // One direct provider per virtual among edges whose conditions must hold.
    let mut provider: HashMap<String, String> = HashMap::new();

    // All direct edges to one name whose conditions must hold point at a single node.
    // Insertion order of first occurrence, like the reference dict of lists.
    let mut groups: Vec<(String, Vec<Bound<'_, PyAny>>)> = Vec::new();

    let mut all_edges = out_edges(slf)?;
    all_edges.extend(out_edges(other)?);
    for edge in all_edges {
        let name = spec_name(&edge_child(&edge)?)?;
        if !edge_direct(&edge)? || name.is_empty() || !must_hold.must_hold(&edge_when(&edge)?)? {
            continue;
        }
        for virtual_name in edge_virtuals(&edge)? {
            let provider_name = provider.entry(virtual_name).or_insert_with(|| name.clone());
            if *provider_name != name {
                return Ok(Some(unsat_dependency_error(py, other, slf)?));
            }
        }
        match groups
            .iter_mut()
            .find(|(group_name, _)| *group_name == name)
        {
            Some((_, group)) => group.push(edge),
            None => groups.push((name, vec![edge])),
        }
    }

    for (_, group) in groups {
        if group.len() < 2 {
            continue;
        }
        // The reference wraps the running merge in `except spack.error.SpecError`.
        let merged: PyResult<Option<Py<PyAny>>> = (|| {
            let mut accumulated: Option<Bound<'_, PyAny>> = None;
            for (i, edge) in group.iter().enumerate().skip(1) {
                let base = accumulated.as_ref().unwrap_or(&group[0]);
                if edge_disjoint_reason(base, edge)?.is_some() {
                    return Ok(Some(unsat_dependency_error(py, other, slf)?));
                }
                // Avoid a copy in the common case of <= 2 edges.
                if i < group.len() - 1 {
                    if accumulated.is_none() {
                        let copy = group[0].call_method0("copy")?;
                        let kwargs = PyDict::new(py);
                        kwargs.set_item("deps", true)?;
                        let child_copy =
                            edge_child(&group[0])?.call_method("copy", (), Some(&kwargs))?;
                        copy.setattr("spec", child_copy)?;
                        accumulated = Some(copy);
                    }
                    edge_merge(accumulated.as_ref().unwrap(), edge)?;
                }
            }
            Ok(None)
        })();
        match merged {
            Ok(Some(error)) => return Ok(Some(error)),
            Ok(None) => {}
            Err(err) if is_spec_error(py, &err)? => {
                return Ok(Some(unsat_dependency_error(py, other, slf)?))
            }
            Err(err) => return Err(err),
        }
    }

    Ok(None)
}

/// `Spec._conflicting_dependencies_reason`.
pub fn conflicting_dependencies_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let py = slf.py();
    if !has_deps(slf)? || !has_deps(other)? {
        return Ok(None);
    }

    let mut must_hold = MustHold::new(slf, other);

    for a in out_edges(slf)? {
        let a_name = spec_name(&edge_child(&a)?)?;
        if !edge_direct(&a)? || a_name.is_empty() {
            continue;
        }
        for b in out_edges(other)? {
            let b_name = spec_name(&edge_child(&b)?)?;
            if !edge_direct(&b)? || b_name.is_empty() {
                continue;
            }
            let a_virtuals = edge_virtuals(&a)?;
            let b_virtuals = edge_virtuals(&b)?;
            let provider_pair =
                a_name != b_name && a_virtuals.iter().any(|v| b_virtuals.contains(v));
            let same_name_pair = a_name == b_name;
            if !provider_pair && !same_name_pair {
                continue;
            }
            if !must_hold.must_hold(&edge_when(&a)?)? || !must_hold.must_hold(&edge_when(&b)?)? {
                continue;
            }
            if provider_pair || edge_disjoint_reason(&a, &b)?.is_some() {
                return Ok(Some(unsat_dependency_error(py, other, slf)?));
            }
        }
    }

    Ok(None)
}

/// `Spec._condition_can_hold`.
pub fn condition_can_hold(slf: &Bound<'_, PyAny>, when: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(conflict_reason(slf, when)?.is_none())
}

// --------------------------------------------------------------------------------------
// Spec: constrain and merge
// --------------------------------------------------------------------------------------

/// `Spec._constrain` (and the `constrain` wrapper).
pub fn spec_constrain(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    deps: bool,
) -> PyResult<bool> {
    let py = slf.py();
    let other = autospec(other)?;

    // A concrete other that already implies self is copied over; self stays a node in its
    // dependents' edge maps, so keep them.
    if is_concrete(&other)? && !is_concrete(slf)? && spec_satisfies(&other, slf, true)? {
        let dependents = slf.getattr("_dependents")?;
        slf.call_method1("_dup", (&other,))?;
        slf.setattr("_dependents", dependents)?;
        return Ok(true);
    }

    if let Some(error) = disjoint_reason(slf, &other, deps)? {
        return Err(PyErr::from_value(error.into_bound(py)));
    }

    // A concrete self that intersects the constraint is a no-op.
    if is_concrete(slf)? {
        return Ok(false);
    }

    spec_merge(slf, &other, deps)
}

/// `Spec._merge`: intersect self with other in place; every step is check-free.
pub fn spec_merge(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
    let py = slf.py();
    let _guard = enter_recursion(py, c" in Spec._merge")?;

    // The precondition compared a concrete other whole, so it satisfies self and is the
    // intersection.
    if is_concrete(other)? && !is_concrete(slf)? {
        let dependents = slf.getattr("_dependents")?;
        slf.call_method1("_dup", (other,))?;
        slf.setattr("_dependents", dependents)?;
        return Ok(true);
    }

    let mut changed = false;

    let self_hash = spec_abstract_hash(slf)?;
    let other_hash = spec_abstract_hash(other)?;
    if truthy_str(&other_hash)
        && (!truthy_str(&self_hash)
            || other_hash.as_deref().unwrap().len() > self_hash.as_deref().unwrap().len())
    {
        slf.setattr("abstract_hash", other_hash)?;
        changed = true;
    }

    let other_name = spec_name(other)?;
    if spec_name(slf)?.is_empty() && !other_name.is_empty() {
        slf.setattr("name", &other_name)?;
        changed = true;
        // The children's dependent edges are filed under the name self had when they were
        // added, the anonymous one: move them to the new name.
        for edge in out_edges(slf)? {
            let dependents = edge_child(&edge)?.getattr("_dependents")?;
            let bucket = dependents.get_item("")?;
            let remaining = siblings_of(&bucket, &edge)?;
            if remaining.len() > 0 {
                dependents.set_item("", remaining)?;
            } else {
                dependents.del_item("")?;
            }
            add_edge_to_map(&dependents, &other_name, &edge)?;
        }
    }

    let other_namespace = spec_namespace(other)?;
    if !truthy_str(&spec_namespace(slf)?) && truthy_str(&other_namespace) {
        slf.setattr("namespace", other_namespace)?;
        changed = true;
    }

    changed |= spec_versions(slf)?
        .call_method1("intersect", (spec_versions(other)?,))?
        .extract::<bool>()?;
    changed |= merge_variants(slf, other)?;
    changed |= spec_compiler_flags(slf)?
        .call_method1("constrain", (spec_compiler_flags(other)?,))?
        .extract::<bool>()?;

    let self_arch = spec_architecture(slf)?;
    let other_arch = spec_architecture(other)?;
    match (self_arch, other_arch) {
        (Some(self_arch), Some(other_arch)) => {
            changed |= self_arch
                .call_method1("_merge", (other_arch,))?
                .extract::<bool>()?;
        }
        (None, Some(other_arch)) => {
            // copy, so that a later merge on self does not write through into other
            slf.setattr("architecture", other_arch.call_method0("copy")?)?;
            changed = true;
        }
        _ => {}
    }

    if deps {
        changed |= merge_dependencies(slf, other)?;
    }

    Ok(changed)
}

/// `Spec._merge_variants`: add the variants of other, merging the ones already present.
pub fn merge_variants(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    let mut changed = false;
    let self_variants = spec_variants(slf)?;
    let other_variants = spec_variants(other)?;
    for item in other_variants.call_method0("items")?.try_iter()? {
        let (name, other_variant): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
        let self_variant = self_variants.call_method1("get", (&name,))?;
        if self_variant.is_none() {
            self_variants.set_item(name, other_variant.call_method0("copy")?)?;
            changed = true;
        } else {
            changed |= self_variant
                .call_method1("_merge", (other_variant,))?
                .extract::<bool>()?;
        }
    }
    Ok(changed)
}

/// `Spec._merge_dependencies`: add the edges of other, merging shared ones, then
/// canonicalize the conditional edges.
pub fn merge_dependencies(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = slf.py();

    // Snapshot of edge identity -> when identity; the handles keep the objects alive so
    // the pointers stay unambiguous.
    fn snapshot<'py>(
        edges: Vec<Bound<'py, PyAny>>,
    ) -> PyResult<Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>> {
        edges
            .into_iter()
            .map(|e| Ok((e.clone(), edge_when(&e)?)))
            .collect()
    }

    let pre = snapshot(out_edges(slf)?)?;
    let mut mutated: HashSet<usize> = HashSet::new();
    let cls = registered(py, &registry::DEPENDENCY_SPEC_CLASS, "DependencySpec class")?;
    for other_edge in out_edges(other)? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("depflag", edge_depflag(&other_edge)?)?;
        kwargs.set_item("virtuals", PyTuple::new(py, edge_virtuals(&other_edge)?)?)?;
        kwargs.set_item("direct", edge_direct(&other_edge)?)?;
        kwargs.set_item("propagation", edge_propagation(&other_edge)?)?;
        // no need to copy the when condition; when conditions are immutable
        kwargs.set_item("when", edge_when(&other_edge)?)?;
        let candidate = cls.call((slf, edge_child(&other_edge)?), Some(&kwargs))?;

        let add_kwargs = PyDict::new(py);
        add_kwargs.set_item("owned", false)?;
        let (edge_changed, survivor): (bool, Bound<'_, PyAny>) = slf
            .call_method("_add_or_merge_edge", (&candidate,), Some(&add_kwargs))?
            .extract()?;
        if edge_changed && !survivor.is(&candidate) {
            mutated.insert(ptr(&survivor));
        }
    }

    canonicalize_conditional_edges(slf)?;

    // An edge added from other and dropped again leaves no trace, so the changed flag
    // comes from what survived: the edge set, the merges into pre-existing edges, and the
    // when conditions the canonicalization removed in place.
    let post = snapshot(out_edges(slf)?)?;
    let pre_map: HashMap<usize, usize> = pre.iter().map(|(e, w)| (ptr(e), ptr(w))).collect();
    let post_map: HashMap<usize, usize> = post.iter().map(|(e, w)| (ptr(e), ptr(w))).collect();
    let pre_keys: HashSet<usize> = pre_map.keys().copied().collect();
    let post_keys: HashSet<usize> = post_map.keys().copied().collect();
    Ok(pre_keys != post_keys
        || mutated.iter().any(|edge_id| post_map.contains_key(edge_id))
        || pre_keys
            .intersection(&post_keys)
            .any(|edge_id| pre_map[edge_id] != post_map[edge_id]))
}

/// `Spec._canonicalize_conditional_edges`: drop falsified conditional edges and make the
/// satisfied ones unconditional.
pub fn canonicalize_conditional_edges(slf: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = slf.py();
    let mut changed = false;
    let mut dirty = true;
    while dirty {
        // A rewrite mutates the edge list, so each round scans a fresh snapshot. Every
        // rewrite reduces the number of conditional edges by at least one, so this ends.
        dirty = false;
        for edge in out_edges(slf)? {
            let when = edge_when(&edge)?;
            if is_empty_spec(&when)? {
                continue;
            }
            if !condition_can_hold(slf, &when)? {
                slf.call_method1("_detach_edge", (&edge,))?;
            } else if condition_must_hold(&when, slf)? {
                slf.call_method1("_detach_edge", (&edge,))?;
                edge.setattr("when", empty_spec_obj(py)?)?;
                slf.call_method1("_add_or_merge_edge", (&edge,))?;
            } else {
                continue;
            }
            changed = true;
            dirty = true;
            break;
        }
    }
    Ok(changed)
}

/// `Spec.constrained`: a constrained copy, without modifying this spec.
pub fn spec_constrained<'py>(
    slf: &Bound<'py, PyAny>,
    other: &Bound<'py, PyAny>,
    deps: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let py = slf.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("deps", deps)?;
    let clone = slf.call_method("copy", (), Some(&kwargs))?;
    clone.call_method1("constrain", (other, deps))?;
    Ok(clone)
}

/// `Spec.intersects`.
pub fn spec_intersects(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
    deps: bool,
) -> PyResult<bool> {
    let other = autospec(other)?;
    Ok(disjoint_reason(slf, &other, deps)?.is_none())
}

// --------------------------------------------------------------------------------------
// DependencySpec: the `_EdgeAlgebra` methods
// --------------------------------------------------------------------------------------

/// Whether the two children are known to point at one node through a virtual, where the
/// node named after the virtual takes the name of the one providing it.
fn edges_renamed(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    let self_name = spec_name(&edge_child(slf)?)?;
    let other_name = spec_name(&edge_child(other)?)?;
    Ok(self_name != other_name
        && (edge_virtuals(other)?.contains(&self_name)
            || edge_virtuals(slf)?.contains(&other_name)))
}

/// `DependencySpec._disjoint_reason`.
pub fn edge_disjoint_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let self_child = edge_child(slf)?;
    let other_child = edge_child(other)?;
    if !edges_renamed(slf, other)? || is_concrete(&self_child)? || is_concrete(&other_child)? {
        return disjoint_reason(&self_child, &other_child, true);
    }
    if let Some(error) = disjoint_node_content_reason(&self_child, &other_child)? {
        return Ok(Some(error));
    }
    disjoint_dependencies_reason(&self_child, &other_child)
}

/// `DependencySpec._conflict_reason`.
pub fn edge_conflict_reason(
    slf: &Bound<'_, PyAny>,
    other: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    let self_child = edge_child(slf)?;
    let other_child = edge_child(other)?;
    if !edges_renamed(slf, other)? || is_concrete(&self_child)? || is_concrete(&other_child)? {
        return conflict_reason(&self_child, &other_child);
    }
    if let Some(error) = disjoint_node_content_reason(&self_child, &other_child)? {
        return Ok(Some(error));
    }
    conflicting_dependencies_reason(&self_child, &other_child)
}

/// `DependencySpec._merge`: merge another edge into this one. Precondition:
/// `_disjoint_reason` or `_conflict_reason` returned None, so this never raises.
pub fn edge_merge(slf: &Bound<'_, PyAny>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = slf.py();
    let _guard = enter_recursion(py, c" in DependencySpec._merge")?;
    let mut changed = false;

    // A node named after a virtual becomes the package providing it, and a node without a
    // name takes the name of the other side; both renames go through the dependents' edge
    // maps.
    let self_child = edge_child(slf)?;
    let other_child = edge_child(other)?;
    let self_name = spec_name(&self_child)?;
    let other_name = spec_name(&other_child)?;
    if edge_virtuals(other)?.contains(&self_name) {
        changed = true;
        rename_node(&self_child, &other_name)?;
    } else if self_name.is_empty() && !other_name.is_empty() {
        changed = true;
        rename_node(&self_child, &other_name)?;
    }

    let self_when = edge_when(slf)?;
    let other_when = edge_when(other)?;
    if self_when.ne(&other_when)? {
        let merged = merged_when(&self_when, &other_when)?;
        if !merged.is(&self_when) {
            changed = true;
            slf.setattr("when", merged)?;
        }
    }

    let kwargs = PyDict::new(py);
    kwargs.set_item("deps", true)?;
    changed |= self_child
        .call_method("_merge", (other_child,), Some(&kwargs))?
        .extract::<bool>()?;
    changed |= slf
        .call_method1("update_deptypes", (edge_depflag(other)?,))?
        .extract::<bool>()?;
    changed |= slf
        .call_method1(
            "update_virtuals",
            (PyTuple::new(py, edge_virtuals(other)?)?,),
        )?
        .extract::<bool>()?;

    if !edge_direct(slf)? && edge_direct(other)? {
        changed = true;
        slf.setattr("direct", true)?;
    }
    let self_propagation = edge_propagation(slf)?;
    let other_propagation = edge_propagation(other)?;
    if self_propagation == PROPAGATION_NONE && other_propagation != self_propagation {
        changed = true;
        slf.setattr("propagation", other_propagation)?;
    }
    Ok(changed)
}

// --------------------------------------------------------------------------------------
// Module-level entry point
// --------------------------------------------------------------------------------------

/// The meet of two specs: a new spec denoting the intersection of the sets they denote,
/// or None when the two are disjoint, since no spec denotes the empty set.
#[pyfunction]
pub fn meet(
    py: Python<'_>,
    a: &Bound<'_, PyAny>,
    b: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyAny>>> {
    match a.call_method1("constrained", (b,)) {
        Ok(constrained) => Ok(Some(constrained.unbind())),
        Err(err) if is_spec_error(py, &err)? => Ok(None),
        Err(err) => Err(err),
    }
}
