// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Objects injected by `spack.spec` at import time. The extension imports nothing
//! from `spack`; everything it needs from the Python side is registered here.

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;

#[allow(dead_code)] // consumed from later milestones' node-construction paths
pub static SPEC_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
#[allow(dead_code)] // consumed from later milestones' `when is EMPTY_SPEC` fast paths
pub static EMPTY_SPEC: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The Python `spack.spec.Spec` subclass; new nodes are constructed through it so
/// they get the subclass `__dict__` attributes and methods.
#[pyfunction]
pub fn register_spec_class(py: Python<'_>, cls: Py<PyAny>) {
    let _ = SPEC_CLASS.set(py, cls);
}

/// The `spack.spec.EMPTY_SPEC` singleton, compared by identity in hot paths.
#[pyfunction]
pub fn register_empty_spec(py: Python<'_>, spec: Py<PyAny>) {
    let _ = EMPTY_SPEC.set(py, spec);
}

#[allow(dead_code)] // registered for completeness; no core error maps to the base class
pub static VERSION_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static EMPTY_RANGE_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static VERSION_LOOKUP_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.version` exception classes, raised by the version bindings so that Python
/// `except` clauses over `spack.error.SpackError` subclasses keep working.
#[pyfunction]
pub fn register_version_errors(
    py: Python<'_>,
    version_error: Py<PyAny>,
    empty_range_error: Py<PyAny>,
    lookup_error: Py<PyAny>,
) {
    let _ = VERSION_ERROR.set(py, version_error);
    let _ = EMPTY_RANGE_ERROR.set(py, empty_range_error);
    let _ = VERSION_LOOKUP_ERROR.set(py, lookup_error);
}

pub static VARIANT_TYPE: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.variant.VariantType` IntEnum; the `VariantValue.type` getter returns its
/// members so that enum identity comparisons on the Python side keep working.
#[pyfunction]
pub fn register_variant_type(py: Python<'_>, cls: Py<PyAny>) {
    let _ = VARIANT_TYPE.set(py, cls);
}

pub static MULTIPLE_VALUES_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static INVALID_VARIANT_VALUE_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static UNSATISFIABLE_VARIANT_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.variant` exception classes, raised by the variant bindings so that Python
/// `except` clauses over `spack.error.SpackError` subclasses keep working.
#[pyfunction]
pub fn register_variant_errors(
    py: Python<'_>,
    multiple_values_error: Py<PyAny>,
    invalid_variant_value_error: Py<PyAny>,
    unsatisfiable_variant_error: Py<PyAny>,
) {
    let _ = MULTIPLE_VALUES_ERROR.set(py, multiple_values_error);
    let _ = INVALID_VARIANT_VALUE_ERROR.set(py, invalid_variant_value_error);
    let _ = UNSATISFIABLE_VARIANT_ERROR.set(py, unsatisfiable_variant_error);
}
