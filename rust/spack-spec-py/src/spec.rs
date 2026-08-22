// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The Rust base class of `spack.spec.Spec`. It owns node-local identity state and
//! the rich-comparison protocol; the Python subclass adds everything not yet ported.

use pyo3::basic::CompareOp;
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
}

impl Spec {
    fn empty() -> Self {
        Spec { name: String::new(), namespace: None, abstract_hash: None, _concrete: false }
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

    /// State held by the Rust struct, merged into the Python `__getstate__` dict.
    fn _rust_state(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let state = PyDict::new(py);
        state.set_item("name", &self.name)?;
        state.set_item("namespace", &self.namespace)?;
        state.set_item("abstract_hash", &self.abstract_hash)?;
        state.set_item("_concrete", self._concrete)?;
        Ok(state.into())
    }
}
