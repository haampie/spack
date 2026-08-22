// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The Rust base class of `spack.spec.DependencySpec`: an edge in the spec DAG. It owns
//! the edge state, the pure state methods, and the edge algebra (`_disjoint_reason`,
//! `_conflict_reason`, `_merge`), implemented in `algebra.rs`.

use pyo3::basic::CompareOp;
use pyo3::exceptions::PyRuntimeError;
use pyo3::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
use pyo3::IntoPyObjectExt;

use crate::lazy::{lazy_eq, lazy_lt};
use crate::registry;

/// `spack.enums.PropagationPolicy.NONE` / `.PREFERENCE` integer values.
const PROPAGATION_NONE: i64 = 1;
const PROPAGATION_PREFERENCE: i64 = 2;

/// The registered `spack.spec.EMPTY_SPEC` singleton.
fn empty_spec(py: Python<'_>) -> PyResult<Py<PyAny>> {
    match registry::EMPTY_SPEC.get(py) {
        Some(spec) => Ok(spec.clone_ref(py)),
        None => Err(PyRuntimeError::new_err(
            "spack_spec: EMPTY_SPEC is not registered",
        )),
    }
}

/// The registered propagation policy value as a `spack.enums.PropagationPolicy` member, or
/// the bare integer when the binding runs without the Python side registered.
fn propagation_to_py(py: Python<'_>, value: i64) -> PyResult<Py<PyAny>> {
    match registry::PROPAGATION_POLICY.get(py) {
        Some(cls) => Ok(cls.bind(py).call1((value,))?.unbind()),
        None => value.into_py_any(py),
    }
}

/// Sorted, deduplicated virtuals as stored: Python `tuple(sorted(set(virtuals)))`.
fn normalize_virtuals(mut virtuals: Vec<String>) -> Vec<String> {
    virtuals.sort_unstable();
    virtuals.dedup();
    virtuals
}

#[pyclass(subclass, name = "DependencySpec", module = "spack_spec")]
pub struct DependencySpec {
    /// Starting node of the edge; `None` only on synthetic edges (e.g. traversal roots).
    #[pyo3(get, set)]
    pub parent: Option<Py<PyAny>>,
    /// Ending node of the edge.
    #[pyo3(get, set)]
    pub spec: Option<Py<PyAny>>,
    /// Dependency type bitset (`spack.deptypes.DepFlag`).
    #[pyo3(get, set)]
    pub depflag: u64,
    pub virtuals: Vec<String>,
    #[pyo3(get, set)]
    pub direct: bool,
    pub propagation: i64,
    /// Condition under which the edge holds; `EMPTY_SPEC` when unconditional.
    #[pyo3(get, set)]
    pub when: Option<Py<PyAny>>,
}

impl DependencySpec {
    /// `self.parent.name if self.parent else None`; likewise for `spec`.
    fn node_name<'py>(py: Python<'py>, node: &Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match node {
            Some(node) => Ok(node.bind(py).getattr("name")?.unbind()),
            None => Ok(py.None()),
        }
    }

    /// The edge-local state compared by `__hash__`: everything but the child node.
    fn hash_tuple<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let items: Vec<Py<PyAny>> = vec![
            Self::node_name(py, &self.parent)?,
            Self::node_name(py, &self.spec)?,
            self.depflag.into_py_any(py)?,
            PyTuple::new(py, &self.virtuals)?.into_py_any(py)?,
            self.direct.into_py_any(py)?,
            propagation_to_py(py, self.propagation)?,
            match &self.when {
                Some(when) => when.clone_ref(py),
                None => py.None(),
            },
        ];
        PyTuple::new(py, items)
    }

    fn eq_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if slf.is(other) {
            return Ok(true);
        }
        if other.is_none() {
            return Ok(false);
        }
        lazy_eq(&slf.getattr("_cmp_iter")?, &other.getattr("_cmp_iter")?)
    }

    fn lt_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if slf.is(other) || other.is_none() {
            return Ok(false);
        }
        lazy_lt(&slf.getattr("_cmp_iter")?, &other.getattr("_cmp_iter")?)
    }

    fn gt_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if slf.is(other) {
            return Ok(false);
        }
        if other.is_none() {
            return Ok(true);
        }
        lazy_lt(&other.getattr("_cmp_iter")?, &slf.getattr("_cmp_iter")?)
    }
}

#[pymethods]
impl DependencySpec {
    /// Mirrors the reference signature `(parent, spec, *, depflag, virtuals, direct=False,
    /// propagation=NONE, when=None)`. All parameters default so that pickle can call
    /// `cls.__new__(cls)` with no arguments before `__setstate__` fills the edge in.
    #[new]
    #[pyo3(signature = (parent=None, spec=None, *, depflag=0, virtuals=None, direct=false, propagation=None, when=None))]
    fn new(
        py: Python<'_>,
        parent: Option<Py<PyAny>>,
        spec: Option<Py<PyAny>>,
        depflag: u64,
        virtuals: Option<Vec<String>>,
        direct: bool,
        propagation: Option<i64>,
        when: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let propagation = propagation.unwrap_or(PROPAGATION_NONE);
        if !direct && propagation != PROPAGATION_NONE {
            return Err(registry::invalid_edge_error(
                py,
                "only direct dependencies can be propagated",
            ));
        }
        // Reference: `self.when = when or EMPTY_SPEC`.
        let when = match when {
            Some(when) if when.bind(py).is_truthy()? => Some(when),
            _ => Some(empty_spec(py)?),
        };
        Ok(DependencySpec {
            parent,
            spec,
            depflag,
            virtuals: normalize_virtuals(virtuals.unwrap_or_default()),
            direct,
            propagation,
            when,
        })
    }

    #[getter]
    fn get_virtuals<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.virtuals)
    }

    #[setter]
    fn set_virtuals(&mut self, virtuals: Vec<String>) {
        self.virtuals = normalize_virtuals(virtuals);
    }

    #[getter]
    fn get_propagation(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        propagation_to_py(py, self.propagation)
    }

    #[setter]
    fn set_propagation(&mut self, value: i64) {
        self.propagation = value;
    }

    /// Why the children of two edges about to merge cannot be a single node, or None when
    /// they can. Ported in `algebra.rs`.
    fn _disjoint_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::edge_disjoint_reason(slf.as_any(), other)
    }

    /// The `Spec._conflict_reason` counterpart of `_disjoint_reason`. Ported in
    /// `algebra.rs`.
    fn _conflict_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::edge_conflict_reason(slf.as_any(), other)
    }

    /// Merge another edge into this one; returns True if the current edge was changed.
    /// Ported in `algebra.rs`.
    fn _merge(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::edge_merge(slf.as_any(), other)
    }

    /// Update the current dependency types.
    fn update_deptypes(&mut self, depflag: u64) -> bool {
        let new = self.depflag | depflag;
        if new == self.depflag {
            return false;
        }
        self.depflag = new;
        true
    }

    /// Update the list of provided virtuals: union with a string or an iterable of strings.
    fn update_virtuals(&mut self, virtuals: &Bound<'_, PyAny>) -> PyResult<bool> {
        let mut incoming: Vec<String> = if let Ok(s) = virtuals.downcast::<PyString>() {
            vec![s.to_cow()?.into_owned()]
        } else {
            virtuals
                .try_iter()?
                .map(|item| item?.extract::<String>())
                .collect::<PyResult<_>>()?
        };
        incoming.extend(self.virtuals.iter().cloned());
        let union = normalize_virtuals(incoming);
        if union.len() == self.virtuals.len() {
            return Ok(false);
        }
        self.virtuals = union;
        Ok(true)
    }

    /// Return a copy of this edge, constructed through the runtime class so that a
    /// Python subclass copies to itself.
    #[pyo3(signature = (*, keep_virtuals=true, keep_parent=true))]
    fn copy(slf: &Bound<'_, Self>, keep_virtuals: bool, keep_parent: bool) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.borrow();
        let parent = if keep_parent {
            match &this.parent {
                Some(parent) => parent.clone_ref(py),
                None => py.None(),
            }
        } else {
            match registry::SPEC_CLASS.get(py) {
                Some(cls) => cls.bind(py).call0()?.unbind(),
                None => {
                    return Err(PyRuntimeError::new_err(
                        "spack_spec: the Spec class is not registered",
                    ))
                }
            }
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item("depflag", this.depflag)?;
        if keep_virtuals {
            kwargs.set_item("virtuals", PyTuple::new(py, &this.virtuals)?)?;
        } else {
            kwargs.set_item("virtuals", PyTuple::empty(py))?;
        }
        kwargs.set_item("propagation", this.propagation)?;
        kwargs.set_item("direct", this.direct)?;
        kwargs.set_item("when", &this.when)?;
        let spec = this.spec.as_ref().map(|s| s.clone_ref(py));
        drop(this);
        Ok(slf.get_type().call((parent, spec), Some(&kwargs))?.unbind())
    }

    /// Flips the dependency and keeps its type. Drops all other information.
    fn flip(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.borrow();
        let parent = this.spec.as_ref().map(|s| s.clone_ref(py));
        let spec = this.parent.as_ref().map(|p| p.clone_ref(py));
        let kwargs = PyDict::new(py);
        kwargs.set_item("depflag", this.depflag)?;
        kwargs.set_item("virtuals", PyTuple::empty(py))?;
        drop(this);
        Ok(slf.get_type().call((parent, spec), Some(&kwargs))?.unbind())
    }

    /// Returns a string, using the spec syntax, representing this edge.
    ///
    /// Args:
    ///     unconditional: if True, removes any condition statement from the representation
    #[pyo3(signature = (*, unconditional=false))]
    fn format(&self, py: Python<'_>, unconditional: bool) -> PyResult<String> {
        let parent = Self::node_or_none(py, &self.parent);
        let child = Self::node_or_none(py, &self.spec);
        let parent_str: String = parent.bind(py).call_method0("format")?.extract()?;
        let child_str: String = child.bind(py).call_method0("format")?.extract()?;

        let virtuals_str = if self.virtuals.is_empty() {
            String::new()
        } else {
            format!("virtuals={}", self.virtuals.join(","))
        };

        // Reference: `self.when != Spec()`; EMPTY_SPEC equals an anonymous Spec by value.
        let mut when_str = String::new();
        if !unconditional && !self.when_is_trivial(py)? {
            let when = self.when.as_ref().expect("when is always assigned");
            when_str = format!("when='{}'", when.bind(py).str()?);
        }

        let dep_sigil = if self.propagation == PROPAGATION_PREFERENCE {
            "%%"
        } else if self.direct {
            "%"
        } else {
            "^"
        };

        let edge_attrs: Vec<&str> = [virtuals_str.as_str(), when_str.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();

        if edge_attrs.is_empty() {
            Ok(format!("{parent_str} {dep_sigil}{child_str}"))
        } else {
            Ok(format!(
                "{parent_str} {dep_sigil}[{}] {child_str}",
                edge_attrs.join(" ")
            ))
        }
    }

    fn __str__(&self, py: Python<'_>) -> PyResult<String> {
        self.format(py, false)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let mut keywords = vec![
            format!("depflag={}", self.depflag),
            format!("virtuals={}", self.get_virtuals(py)?.repr()?),
        ];
        if self.direct {
            keywords.push("direct=True".to_string());
        }
        if !self.when_is_trivial(py)? {
            let when = self.when.as_ref().expect("when is always assigned");
            keywords.push(format!("when={}", when.bind(py).str()?));
        }
        if self.propagation != PROPAGATION_NONE {
            let name: String = propagation_to_py(py, self.propagation)?
                .bind(py)
                .getattr("name")?
                .extract()?;
            keywords.push(format!("propagation=PropagationPolicy.{name}"));
        }
        let parent = Self::node_or_none(py, &self.parent);
        let child = Self::node_or_none(py, &self.spec);
        let parent_repr = parent.bind(py).call_method0("format")?.repr()?;
        let child_repr = child.bind(py).call_method0("format")?.repr()?;
        Ok(format!(
            "DependencySpec({parent_repr}, {child_repr}, {})",
            keywords.join(", ")
        ))
    }

    /// Lazily comparable edge state; the child node is the tie-breaker for parallel edges
    /// (`^foo@1 ^foo+bar`).
    fn _cmp_iter<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let items: Vec<Py<PyAny>> = vec![
            Self::node_name(py, &self.parent)?,
            Self::node_name(py, &self.spec)?,
            self.depflag.into_py_any(py)?,
            PyTuple::new(py, &self.virtuals)?.into_py_any(py)?,
            self.direct.into_py_any(py)?,
            propagation_to_py(py, self.propagation)?,
            match &self.when {
                Some(when) => when.clone_ref(py),
                None => py.None(),
            },
            match &self.spec {
                Some(spec) => spec.clone_ref(py),
                None => py.None(),
            },
        ];
        PyTuple::new(py, items)
    }

    /// Hash edge properties, do not include the node.
    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        self.hash_tuple(py)?.hash()
    }

    fn __richcmp__(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        op: CompareOp,
    ) -> PyResult<bool> {
        match op {
            CompareOp::Eq => Self::eq_impl(slf, other),
            CompareOp::Ne => Ok(!Self::eq_impl(slf, other)?),
            CompareOp::Lt => Self::lt_impl(slf, other),
            CompareOp::Gt => Self::gt_impl(slf, other),
            CompareOp::Le => Ok(!Self::gt_impl(slf, other)?),
            CompareOp::Ge => Ok(!Self::lt_impl(slf, other)?),
        }
    }

    /// Pickle state in the reference format: the `(None, slots_dict)` pair the default
    /// slots-class pickling produces, so pickles interchange between implementations.
    fn __getstate__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let slots = PyDict::new(py);
        slots.set_item("parent", &self.parent)?;
        slots.set_item("spec", &self.spec)?;
        slots.set_item("depflag", self.depflag)?;
        slots.set_item("virtuals", self.get_virtuals(py)?)?;
        slots.set_item("direct", self.direct)?;
        slots.set_item("propagation", propagation_to_py(py, self.propagation)?)?;
        slots.set_item("when", &self.when)?;
        PyTuple::new(py, [py.None(), slots.into_py_any(py)?])
    }

    fn __setstate__(&mut self, py: Python<'_>, state: &Bound<'_, PyAny>) -> PyResult<()> {
        // Accept both the `(dict_state, slots_dict)` pair and a bare dict.
        let slots: Bound<'_, PyDict> = if let Ok(pair) = state.downcast::<PyTuple>() {
            pair.get_item(1)?.extract()?
        } else {
            state.extract()?
        };
        let get = |key: &str| -> PyResult<Option<Py<PyAny>>> {
            Ok(slots.get_item(key)?.map(|v| v.unbind()))
        };
        self.parent = get("parent")?.filter(|v| !v.is_none(py));
        self.spec = get("spec")?.filter(|v| !v.is_none(py));
        self.when = get("when")?.filter(|v| !v.is_none(py));
        if let Some(depflag) = slots.get_item("depflag")? {
            self.depflag = depflag.extract()?;
        }
        if let Some(virtuals) = slots.get_item("virtuals")? {
            self.virtuals = normalize_virtuals(virtuals.extract()?);
        }
        if let Some(direct) = slots.get_item("direct")? {
            self.direct = direct.extract()?;
        }
        if let Some(propagation) = slots.get_item("propagation")? {
            self.propagation = propagation.extract()?;
        }
        Ok(())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(parent) = &self.parent {
            visit.call(parent)?;
        }
        if let Some(spec) = &self.spec {
            visit.call(spec)?;
        }
        if let Some(when) = &self.when {
            visit.call(when)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        self.parent = None;
        self.spec = None;
        self.when = None;
    }
}

impl DependencySpec {
    /// A stored node handle or Python `None`, for call sites that mirror reference code
    /// accessing `self.parent` / `self.spec` unconditionally.
    fn node_or_none(py: Python<'_>, node: &Option<Py<PyAny>>) -> Py<PyAny> {
        match node {
            Some(node) => node.clone_ref(py),
            None => py.None(),
        }
    }

    /// Whether `when` equals an anonymous `Spec()`, via the `EMPTY_SPEC` singleton.
    fn when_is_trivial(&self, py: Python<'_>) -> PyResult<bool> {
        let when = match &self.when {
            Some(when) => when.bind(py).clone(),
            None => return Ok(true),
        };
        if let Some(empty) = registry::EMPTY_SPEC.get(py) {
            when.eq(empty.bind(py))
        } else {
            Err(PyRuntimeError::new_err(
                "spack_spec: EMPTY_SPEC is not registered",
            ))
        }
    }
}
