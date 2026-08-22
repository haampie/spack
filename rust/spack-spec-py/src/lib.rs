// SPDX-License-Identifier: (Apache-2.0 OR MIT)

use pyo3::prelude::*;

mod lazy;
mod registry;
mod spec;

#[pymodule]
fn spack_spec(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<spec::Spec>()?;
    m.add_function(wrap_pyfunction!(registry::register_spec_class, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_empty_spec, m)?)?;
    Ok(())
}
