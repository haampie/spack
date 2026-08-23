// SPDX-License-Identifier: (Apache-2.0 OR MIT)

use pyo3::prelude::*;

mod algebra;
mod arch;
mod asp;
mod cmp;
mod edge;
mod fmt;
mod graph;
mod lazy;
mod parser;
mod registry;
mod spec;
mod variant;
mod version;

#[pymodule]
fn spack_spec(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<spec::Spec>()?;
    m.add_class::<edge::DependencySpec>()?;
    m.add_class::<version::VersionType>()?;
    m.add_class::<version::ConcreteVersion>()?;
    m.add_class::<version::StandardVersion>()?;
    m.add_class::<version::GitVersion>()?;
    m.add_class::<version::ClosedOpenRange>()?;
    m.add_class::<version::VersionList>()?;
    m.add_function(wrap_pyfunction!(version::version_factory, m)?)?;
    m.add_function(wrap_pyfunction!(version::version_range_factory, m)?)?;
    m.add_function(wrap_pyfunction!(version::from_string, m)?)?;
    m.add_function(wrap_pyfunction!(version::ver, m)?)?;
    m.add_function(wrap_pyfunction!(version::next_version_py, m)?)?;
    m.add_function(wrap_pyfunction!(version::prev_version_py, m)?)?;
    m.add_class::<variant::VariantValue>()?;
    m.add_function(wrap_pyfunction!(variant::render_variant_map, m)?)?;
    m.add_class::<arch::ArchSpec>()?;
    m.add_class::<asp::AspVar>()?;
    m.add_class::<asp::AspFunction>()?;
    m.add_class::<asp::ProblemInstanceBuilder>()?;
    m.add_function(wrap_pyfunction!(algebra::meet, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_spec_class, m)?)?;
    m.add_function(wrap_pyfunction!(
        registry::register_dependency_spec_class,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(registry::register_any_version, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_algebra_errors, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_empty_spec, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_arch_oracle, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_arch_errors, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_version_errors, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_variant_type, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_propagation_policy, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_edge_errors, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_variant_errors, m)?)?;
    m.add_function(wrap_pyfunction!(
        registry::register_deptype_canonicalize,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(registry::register_hash_descriptors, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_parser_helpers, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_format_helpers, m)?)?;
    m.add_function(wrap_pyfunction!(registry::register_parse_callbacks, m)?)?;
    m.add_function(wrap_pyfunction!(parser::parse_one_or_raise, m)?)?;
    Ok(())
}
