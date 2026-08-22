// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The Rust base class of `spack.spec.Spec`. It owns node-local identity state, the
//! rich-comparison protocol, and the satisfies/intersects/constrain algebra (implemented
//! in `algebra.rs`); the Python subclass adds everything not yet ported.

use pyo3::basic::CompareOp;
use pyo3::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use pyo3::IntoPyObjectExt;

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
        if other.is_instance_of::<Spec>() {
            return crate::cmp::spec_lazy_eq(slf.as_any(), other);
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
        if other.is_instance_of::<Spec>() {
            return crate::cmp::spec_lazy_lt(slf.as_any(), other);
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
        if other.is_instance_of::<Spec>() {
            return crate::cmp::spec_lazy_lt(other, slf.as_any());
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

    // ----------------------------------------------------------------------------------
    // Construction and mutation path, ported in `graph.rs`.
    // ----------------------------------------------------------------------------------

    /// Called by the parser to add a known flag.
    fn _add_flag(
        slf: &Bound<'_, Self>,
        name: &str,
        value: &Bound<'_, PyAny>,
        propagate: bool,
        concrete: bool,
    ) -> PyResult<()> {
        crate::graph::add_flag(slf.as_any(), name, value, propagate, concrete)
    }

    /// Called by the parser to set the architecture.
    #[pyo3(signature = (**kwargs))]
    fn _set_architecture(
        slf: &Bound<'_, Self>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        crate::graph::set_architecture(slf.as_any(), kwargs)
    }

    /// Called by the parser to add another spec as a dependency.
    #[pyo3(signature = (spec, *, depflag, virtuals, direct=false, propagation=None, when=None))]
    fn _add_dependency(
        slf: &Bound<'_, Self>,
        spec: &Bound<'_, PyAny>,
        depflag: u64,
        virtuals: &Bound<'_, PyAny>,
        direct: bool,
        propagation: Option<&Bound<'_, PyAny>>,
        when: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        // Dispatch like the reference `self.add_dependency_edge(...)`: the
        // `_ImmutableSpec` override guards mutation.
        let py = slf.py();
        let kwargs = PyDict::new(py);
        kwargs.set_item("depflag", depflag)?;
        kwargs.set_item("virtuals", virtuals)?;
        kwargs.set_item("direct", direct)?;
        if let Some(propagation) = propagation {
            kwargs.set_item("propagation", propagation)?;
        }
        kwargs.set_item("when", when)?;
        slf.call_method("add_dependency_edge", (spec,), Some(&kwargs))?;
        Ok(())
    }

    /// Add a dependency edge to this spec.
    #[pyo3(signature = (dependency_spec, *, depflag, virtuals, direct=false, propagation=None, when=None))]
    fn add_dependency_edge(
        slf: &Bound<'_, Self>,
        dependency_spec: &Bound<'_, PyAny>,
        depflag: u64,
        virtuals: &Bound<'_, PyAny>,
        direct: bool,
        propagation: Option<&Bound<'_, PyAny>>,
        when: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        crate::graph::add_dependency_edge(
            slf.as_any(),
            dependency_spec,
            depflag,
            virtuals,
            direct,
            propagation,
            when,
        )
    }

    /// Add `candidate` as a dependency edge; returns whether self changed and the edge
    /// that carries the candidate's constraint afterwards.
    #[pyo3(signature = (candidate, owned=true))]
    fn _add_or_merge_edge(
        slf: &Bound<'_, Self>,
        candidate: &Bound<'_, PyAny>,
        owned: bool,
    ) -> PyResult<(bool, Py<PyAny>)> {
        let (changed, edge) = crate::graph::add_or_merge_edge(slf.as_any(), candidate, owned)?;
        Ok((changed, edge.unbind()))
    }

    /// Remove an edge from this spec and from the dependents of the node it points at.
    fn _detach_edge(slf: &Bound<'_, Self>, edge: &Bound<'_, PyAny>) -> PyResult<()> {
        crate::graph::detach_edge(slf.as_any(), edge)
    }

    /// Trim the dependencies of this spec.
    fn clear_dependencies(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_dependencies")?.call_method0("clear")?;
        Ok(())
    }

    /// Trim the dependencies and dependents of this spec.
    fn clear_edges(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_dependencies")?.call_method0("clear")?;
        slf.getattr("_dependents")?.call_method0("clear")?;
        Ok(())
    }

    /// Return a list of edges connecting this node in the DAG to parents.
    #[pyo3(signature = (name=None, depflag=crate::algebra::DT_ALL, *, virtuals=None))]
    fn edges_from_dependents(
        slf: &Bound<'_, Self>,
        name: Option<String>,
        depflag: u64,
        virtuals: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let edges = crate::graph::select_edges(
            &slf.getattr("_dependents")?,
            name.as_deref(),
            None,
            depflag,
            virtuals,
        )?;
        Ok(edges.into_iter().map(Bound::unbind).collect())
    }

    /// Returns a list of edges connecting this node in the DAG to children.
    #[pyo3(signature = (name=None, depflag=crate::algebra::DT_ALL, *, virtuals=None))]
    fn edges_to_dependencies(
        slf: &Bound<'_, Self>,
        name: Option<String>,
        depflag: u64,
        virtuals: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let edges = crate::graph::select_edges(
            &slf.getattr("_dependencies")?,
            None,
            name.as_deref(),
            depflag,
            virtuals,
        )?;
        Ok(edges.into_iter().map(Bound::unbind).collect())
    }

    /// Returns a list of direct dependencies (nodes in the DAG).
    #[pyo3(signature = (name=None, deptype=None, *, virtuals=None))]
    fn dependencies(
        slf: &Bound<'_, Self>,
        name: Option<String>,
        deptype: Option<&Bound<'_, PyAny>>,
        virtuals: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let depflag = match deptype {
            Some(deptype) => crate::graph::canonical_depflag(deptype)?,
            None => crate::algebra::DT_ALL,
        };
        let edges = crate::graph::select_edges(
            &slf.getattr("_dependencies")?,
            None,
            name.as_deref(),
            depflag,
            virtuals,
        )?;
        edges
            .into_iter()
            .map(|e| Ok(e.getattr("spec")?.unbind()))
            .collect()
    }

    /// Return a list of direct dependents (nodes in the DAG).
    #[pyo3(signature = (name=None, deptype=None))]
    fn dependents(
        slf: &Bound<'_, Self>,
        name: Option<String>,
        deptype: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let depflag = match deptype {
            Some(deptype) => crate::graph::canonical_depflag(deptype)?,
            None => crate::algebra::DT_ALL,
        };
        let edges = crate::graph::select_edges(
            &slf.getattr("_dependents")?,
            name.as_deref(),
            None,
            depflag,
            None,
        )?;
        edges
            .into_iter()
            .map(|e| Ok(e.getattr("parent")?.unbind()))
            .collect()
    }

    /// The single edge to the dependency of the given name (original-concretizer detail).
    fn _get_dependency(slf: &Bound<'_, Self>, name: &str) -> PyResult<Py<PyAny>> {
        Ok(crate::graph::get_dependency(slf.as_any(), name)?.unbind())
    }

    // ----------------------------------------------------------------------------------
    // Copy path, ported in `graph.rs`.
    // ----------------------------------------------------------------------------------

    /// Copies `other` into self, by overwriting all attributes; returns True if self
    /// changed because of the copy operation.
    #[pyo3(signature = (other, deps=None, *, propagation=None))]
    fn _dup(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        deps: Option<&Bound<'_, PyAny>>,
        propagation: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let py = slf.py();
        let deps: Bound<'_, PyAny> = match deps {
            Some(deps) => deps.clone(),
            None => true.into_bound_py_any(py)?,
        };
        crate::graph::spec_dup(slf.as_any(), other, &deps, propagation)
    }

    /// Copy the edges of `other` verbatim onto fresh node copies.
    #[pyo3(signature = (other, depflag, propagation=None))]
    fn _dup_deps(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        depflag: u64,
        propagation: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        crate::graph::spec_dup_deps(slf.as_any(), other, depflag, propagation)
    }

    /// Make a copy of this spec, optionally restricting the copied dependency types.
    #[pyo3(signature = (deps=None, **kwargs))]
    fn copy(
        slf: &Bound<'_, Self>,
        deps: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        Ok(crate::graph::spec_copy(slf.as_any(), deps, kwargs)?.unbind())
    }

    // ----------------------------------------------------------------------------------
    // Canonical comparison stream, ported in `cmp.rs`.
    // ----------------------------------------------------------------------------------

    /// Comparable elements of just this node and not its deps, in the reference order.
    fn _cmp_node(slf: &Bound<'_, Self>) -> PyResult<Py<PyTuple>> {
        let py = slf.py();
        let mut items: Vec<Py<PyAny>> = {
            let this = slf.borrow();
            vec![
                this.name.clone().into_py_any(py)?,
                this.namespace.clone().into_py_any(py)?,
                match &this.versions {
                    Some(handle) => handle.clone_ref(py),
                    None => py.None(),
                },
                match &this.variants {
                    Some(handle) => handle.clone_ref(py),
                    None => py.None(),
                },
                match &this.compiler_flags {
                    Some(handle) => handle.clone_ref(py),
                    None => py.None(),
                },
                match &this.architecture {
                    Some(handle) => handle.clone_ref(py),
                    None => py.None(),
                },
                this.abstract_hash.clone().into_py_any(py)?,
            ]
        };
        // this is not present on older specs
        items.push(crate::graph::getattr_or_none(slf.as_any(), "_package_hash")?.unbind());
        Ok(PyTuple::new(py, items)?.unbind())
    }

    /// The lazily comparable `(nodes, edges)` streams of self.
    fn _cmp_iter(slf: &Bound<'_, Self>) -> PyResult<Py<PyTuple>> {
        crate::cmp::make_cmp_iter(slf.as_any())
    }

    // ----------------------------------------------------------------------------------
    // Default formatting fast paths, ported in `fmt.rs`.
    // ----------------------------------------------------------------------------------

    /// Fast path for formatting with DEFAULT_FORMAT and no color.
    fn _format_default(slf: &Bound<'_, Self>) -> PyResult<String> {
        crate::fmt::format_default(slf.as_any())
    }

    /// Render the deptypes/when/virtuals attributes of an edge.
    #[pyo3(signature = (dep, deptypes=true, virtuals=true))]
    fn _format_edge_attributes(
        slf: &Bound<'_, Self>,
        dep: &Bound<'_, PyAny>,
        deptypes: bool,
        virtuals: bool,
    ) -> PyResult<String> {
        let _ = slf;
        crate::fmt::format_edge_attributes(dep, deptypes, virtuals)
    }

    /// Helper for formatting dependencies on specs.
    #[pyo3(signature = (format_string=None, include=None, deptypes=true, color=Some(false), _force_direct=false))]
    fn _format_dependencies(
        slf: &Bound<'_, Self>,
        format_string: Option<&Bound<'_, PyAny>>,
        include: Option<&Bound<'_, PyAny>>,
        deptypes: bool,
        color: Option<bool>,
        _force_direct: bool,
    ) -> PyResult<String> {
        crate::fmt::format_dependencies(
            slf.as_any(),
            format_string,
            include,
            deptypes,
            color,
            _force_direct,
        )
    }

    /// Helper for `long_spec` and `clong_spec`.
    #[pyo3(signature = (color=Some(false)))]
    fn _long_spec(slf: &Bound<'_, Self>, color: Option<bool>) -> PyResult<String> {
        crate::fmt::long_spec(slf.as_any(), color)
    }

    /// String representation of this spec.
    #[pyo3(signature = (color=Some(false)))]
    fn _str(slf: &Bound<'_, Self>, color: Option<bool>) -> PyResult<String> {
        crate::fmt::str_impl(slf.as_any(), color)
    }

    fn __str__(slf: &Bound<'_, Self>) -> PyResult<String> {
        crate::fmt::str_impl(slf.as_any(), Some(false))
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
