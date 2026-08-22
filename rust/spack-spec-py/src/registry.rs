// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Objects injected by `spack.spec` at import time. The extension imports nothing
//! from `spack`; everything it needs from the Python side is registered here.

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;

pub static SPEC_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
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

pub static PROPAGATION_POLICY: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.enums.PropagationPolicy` IntEnum; the edge `propagation` getter returns its
/// members so that Python-side enum identity comparisons keep working.
#[pyfunction]
pub fn register_propagation_policy(py: Python<'_>, cls: Py<PyAny>) {
    let _ = PROPAGATION_POLICY.set(py, cls);
}

pub static INVALID_EDGE_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.spec.InvalidEdgeError` class, raised by the `DependencySpec` constructor so
/// that Python `except` clauses over `spack.error.SpecError` subclasses keep working.
#[pyfunction]
pub fn register_edge_errors(py: Python<'_>, invalid_edge_error: Py<PyAny>) {
    let _ = INVALID_EDGE_ERROR.set(py, invalid_edge_error);
}

/// An `InvalidEdgeError` with the given message, or `ValueError` when the Python side has
/// not registered the class.
pub fn invalid_edge_error(py: Python<'_>, message: &str) -> PyErr {
    match INVALID_EDGE_ERROR.get(py) {
        Some(cls) => match cls.bind(py).call1((message,)) {
            Ok(exc) => PyErr::from_value(exc),
            Err(e) => e,
        },
        None => pyo3::exceptions::PyValueError::new_err(message.to_string()),
    }
}

pub static ARCH_ORACLE: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.spec._ArchOracle` instance: late-bound access to `spack.platforms` and the
/// archspec target table, so `use_platform` and monkeypatched modules are observed live.
#[pyfunction]
pub fn register_arch_oracle(py: Python<'_>, oracle: Py<PyAny>) {
    let _ = ARCH_ORACLE.set(py, oracle);
}

pub static UNSAT_ARCH_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.spec.UnsatisfiableArchitectureSpecError` class, raised by `ArchSpec.constrain`
/// so that Python `except` clauses over `spack.error.SpackError` subclasses keep working.
#[pyfunction]
pub fn register_arch_errors(py: Python<'_>, unsatisfiable_arch_error: Py<PyAny>) {
    let _ = UNSAT_ARCH_ERROR.set(py, unsatisfiable_arch_error);
}

pub static DEPENDENCY_SPEC_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The Python `spack.spec.DependencySpec` subclass; the algebra constructs candidate and
/// synthetic edges through it so they get the subclass behavior.
#[pyfunction]
pub fn register_dependency_spec_class(py: Python<'_>, cls: Py<PyAny>) {
    let _ = DEPENDENCY_SPEC_CLASS.set(py, cls);
}

pub static ANY_VERSION: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The `spack.version.any_version` singleton, compared against by the edge satisfaction
/// checks exactly like the reference implementation does.
#[pyfunction]
pub fn register_any_version(py: Python<'_>, any_version: Py<PyAny>) {
    let _ = ANY_VERSION.set(py, any_version);
}

pub static SPEC_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static UNSATISFIABLE_SPEC_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static UNSAT_SPEC_NAME_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static UNSAT_VERSION_SPEC_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static UNSAT_DEPENDENCY_SPEC_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
pub static INVALID_HASH_ERROR: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

/// The exception classes the constrain/intersects/satisfies algebra returns and raises:
/// `spack.error.SpecError`, `spack.error.UnsatisfiableSpecError`, and the `spack.spec`
/// subclasses for name, version, dependency and hash mismatches.
#[pyfunction]
pub fn register_algebra_errors(
    py: Python<'_>,
    spec_error: Py<PyAny>,
    unsatisfiable_spec_error: Py<PyAny>,
    name_error: Py<PyAny>,
    version_error: Py<PyAny>,
    dependency_error: Py<PyAny>,
    invalid_hash_error: Py<PyAny>,
) {
    let _ = SPEC_ERROR.set(py, spec_error);
    let _ = UNSATISFIABLE_SPEC_ERROR.set(py, unsatisfiable_spec_error);
    let _ = UNSAT_SPEC_NAME_ERROR.set(py, name_error);
    let _ = UNSAT_VERSION_SPEC_ERROR.set(py, version_error);
    let _ = UNSAT_DEPENDENCY_SPEC_ERROR.set(py, dependency_error);
    let _ = INVALID_HASH_ERROR.set(py, invalid_hash_error);
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
