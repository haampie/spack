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
