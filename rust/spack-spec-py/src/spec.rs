// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The Rust base class of `spack.spec.Spec`. It owns node-local identity state, the
//! rich-comparison protocol, and the satisfies/intersects/constrain algebra (implemented
//! in `algebra.rs`); the Python subclass adds everything not yet ported.

use pyo3::basic::CompareOp;
use pyo3::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use crate::lazy::{lazy_eq, lazy_lt};

#[pyclass(subclass, name = "Spec", module = "spack_spec")]
pub struct Spec {
    /// Package name; the empty string on anonymous specs.
    #[pyo3(get, set)]
    pub name: String,
    #[pyo3(get, set)]
    pub namespace: Option<String>,
    #[pyo3(get, set)]
    pub abstract_hash: Option<String>,
    #[pyo3(get, set)]
    pub _concrete: bool,
    // Python-side containers, held as opaque handles: the Python subclass assigns and
    // mutates them, the getters return them identically.
    /// A `spack.version.VersionList`.
    #[pyo3(get, set)]
    pub versions: Option<Py<PyAny>>,
    /// A `spack.spec.VariantMap`.
    #[pyo3(get, set)]
    pub variants: Option<Py<PyAny>>,
    /// A `spack.spec.FlagMap`.
    #[pyo3(get, set)]
    pub compiler_flags: Option<Py<PyAny>>,
    /// An `ArchSpec`, or None.
    #[pyo3(get, set)]
    pub architecture: Option<Py<PyAny>>,
    /// Edge map `dict[str, list[DependencySpec]]` of outgoing edges, keyed by child name.
    #[pyo3(get, set)]
    pub _dependencies: Option<Py<PyAny>>,
    /// Edge map `dict[str, list[DependencySpec]]` of incoming edges, keyed by parent name.
    #[pyo3(get, set)]
    pub _dependents: Option<Py<PyAny>>,
    /// Virtual name -> `VersionList` provided, frozen at concretization; None on abstract
    /// specs.
    #[pyo3(get, set)]
    pub _provided_virtuals: Option<Py<PyAny>>,
}

impl Spec {
    fn empty() -> Self {
        Spec {
            name: String::new(),
            namespace: None,
            abstract_hash: None,
            _concrete: false,
            versions: None,
            variants: None,
            compiler_flags: None,
            architecture: None,
            _dependencies: None,
            _dependents: None,
            _provided_virtuals: None,
        }
    }

    /// `_cmp_fast_eq` on the Python subclass: True/False short-circuit or None.
    fn fast_eq(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
        slf.call_method1("_cmp_fast_eq", (other,))?.extract()
    }

    fn eq_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Some(verdict) = Self::fast_eq(slf, other)? {
            return Ok(verdict);
        }
        if other.is_none() {
            return Ok(false);
        }
        lazy_eq(&slf.getattr("_cmp_iter")?, &other.getattr("_cmp_iter")?)
    }

    fn lt_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if Self::fast_eq(slf, other)? == Some(true) {
            return Ok(false);
        }
        if other.is_none() {
            return Ok(false);
        }
        lazy_lt(&slf.getattr("_cmp_iter")?, &other.getattr("_cmp_iter")?)
    }

    fn gt_impl(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if Self::fast_eq(slf, other)? == Some(true) {
            return Ok(false);
        }
        if other.is_none() {
            return Ok(true);
        }
        lazy_lt(&other.getattr("_cmp_iter")?, &slf.getattr("_cmp_iter")?)
    }
}

#[pymethods]
impl Spec {
    /// Arguments are consumed by the Python subclass `__init__`.
    #[new]
    #[pyo3(signature = (*args, **kwargs))]
    fn new(args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        let _ = (args, kwargs);
        Self::empty()
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

    // ----------------------------------------------------------------------------------
    // The satisfies/intersects/constrain algebra, ported in `algebra.rs`. Signatures
    // mirror the Python reference methods deleted from the subclass in rust mode.
    // ----------------------------------------------------------------------------------

    /// Constrains self with other, and returns True if self changed, False otherwise.
    #[pyo3(signature = (other, deps=true))]
    fn constrain(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
        crate::algebra::spec_constrain(slf.as_any(), other, deps)
    }

    #[pyo3(signature = (other, deps=true))]
    fn _constrain(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
        crate::algebra::spec_constrain(slf.as_any(), other, deps)
    }

    /// Return None when at least one concrete spec matches both self and other, otherwise
    /// the reason the two are disjoint.
    #[pyo3(signature = (other, deps))]
    fn _disjoint_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        deps: bool,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::disjoint_reason(slf.as_any(), other, deps)
    }

    /// Return None unless self and other conflict with each other, otherwise the reason.
    fn _conflict_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::conflict_reason(slf.as_any(), other)
    }

    fn _disjoint_node_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::disjoint_node_reason(slf.as_any(), other)
    }

    fn _disjoint_node_content_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::disjoint_node_content_reason(slf.as_any(), other)
    }

    fn _disjoint_node_attributes_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::disjoint_node_attributes_reason(slf.as_any(), other)
    }

    fn _disjoint_dependencies_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::disjoint_dependencies_reason(slf.as_any(), other)
    }

    fn _conflicting_dependencies_reason(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        crate::algebra::conflicting_dependencies_reason(slf.as_any(), other)
    }

    /// Intersect self with other in place, and return True iff self changed.
    #[pyo3(signature = (other, deps))]
    fn _merge(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
        crate::algebra::spec_merge(slf.as_any(), other, deps)
    }

    fn _merge_variants(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::merge_variants(slf.as_any(), other)
    }

    fn _merge_dependencies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::merge_dependencies(slf.as_any(), other)
    }

    fn _canonicalize_conditional_edges(slf: &Bound<'_, Self>) -> PyResult<bool> {
        crate::algebra::canonicalize_conditional_edges(slf.as_any())
    }

    /// Return a constrained copy without modifying this spec.
    #[pyo3(signature = (other, deps=true))]
    fn constrained(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        deps: bool,
    ) -> PyResult<Py<PyAny>> {
        Ok(crate::algebra::spec_constrained(slf.as_any(), other, deps)?.unbind())
    }

    /// Used to convert arguments to specs: a spec passes through, a string is parsed.
    fn _autospec(slf: &Bound<'_, Self>, spec_like: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let _ = slf;
        Ok(crate::algebra::autospec(spec_like)?.unbind())
    }

    /// Return True if there exists at least one concrete spec that matches both self and
    /// other, otherwise False.
    #[pyo3(signature = (other, deps=true))]
    fn intersects(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
        crate::algebra::spec_intersects(slf.as_any(), other, deps)
    }

    /// Whether the when condition of an edge can hold for this spec.
    fn _condition_can_hold(slf: &Bound<'_, Self>, when: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::condition_can_hold(slf.as_any(), when)
    }

    /// Return True if all concrete specs matching self also match other, otherwise False.
    #[pyo3(signature = (other, deps=true))]
    fn satisfies(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, deps: bool) -> PyResult<bool> {
        crate::algebra::spec_satisfies(slf.as_any(), other, deps)
    }

    /// Return True if this spec provides the given virtual spec, using the provided
    /// virtual versions frozen on the node.
    fn _provides_virtual(slf: &Bound<'_, Self>, virtual_spec: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::provides_virtual(slf.as_any(), virtual_spec)
    }

    /// Compares self and other without looking at dependencies.
    fn _satisfies_node(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::satisfies_node(slf.as_any(), other)
    }

    fn _satisfies_variants(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        crate::algebra::satisfies_variants(slf.as_any(), other)
    }

    fn _satisfies_variants_when_self_concrete(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        crate::algebra::satisfies_variants_when_self_concrete(slf.as_any(), other)
    }

    fn _satisfies_variants_when_self_abstract(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        crate::algebra::satisfies_variants_when_self_abstract(slf.as_any(), other)
    }

    /// State held by the Rust struct, merged into the Python `__getstate__` dict.
    fn _rust_state(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let state = PyDict::new(py);
        state.set_item("name", &self.name)?;
        state.set_item("namespace", &self.namespace)?;
        state.set_item("abstract_hash", &self.abstract_hash)?;
        state.set_item("_concrete", self._concrete)?;
        state.set_item("versions", &self.versions)?;
        state.set_item("variants", &self.variants)?;
        state.set_item("compiler_flags", &self.compiler_flags)?;
        state.set_item("architecture", &self.architecture)?;
        state.set_item("_dependencies", &self._dependencies)?;
        state.set_item("_dependents", &self._dependents)?;
        state.set_item("_provided_virtuals", &self._provided_virtuals)?;
        Ok(state.into())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for handle in [
            &self.versions,
            &self.variants,
            &self.compiler_flags,
            &self.architecture,
            &self._dependencies,
            &self._dependents,
            &self._provided_virtuals,
        ] {
            if let Some(handle) = handle {
                visit.call(handle)?;
            }
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        self.versions = None;
        self.variants = None;
        self.compiler_flags = None;
        self.architecture = None;
        self._dependencies = None;
        self._dependents = None;
        self._provided_virtuals = None;
    }
}
