// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! The default formatting fast paths of `spack.spec.Spec`: `_format_default`, the
//! dependency formatting helpers, and the `__str__` family. Transcribed from the reference
//! implementation in `lib/spack/spack/spec.py`; the general `format()` and the concrete
//! `tree()` rendering stay in Python and are reached through method dispatch.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

use crate::algebra::{
    edge_child, edge_depflag, edge_direct, edge_propagation, edge_virtuals, edge_when,
    empty_spec_obj, has_deps, is_concrete, out_edges, registered, spec_architecture,
    spec_compiler_flags, spec_name, spec_namespace, spec_variants, spec_versions, truthy_str,
    PROPAGATION_PREFERENCE,
};
use crate::registry;
use crate::spec::Spec;

/// `dt.flag_to_tuple`, transcribed from `lib/spack/spack/deptypes.py`.
fn flag_to_tuple(depflag: u64) -> Vec<&'static str> {
    let mut deptype = Vec::new();
    if depflag & crate::algebra::DT_BUILD != 0 {
        deptype.push("build");
    }
    if depflag & crate::algebra::DT_LINK != 0 {
        deptype.push("link");
    }
    if depflag & crate::algebra::DT_RUN != 0 {
        deptype.push("run");
    }
    if depflag & crate::algebra::DT_TEST != 0 {
        deptype.push("test");
    }
    deptype
}

/// `str(obj)` as a Rust string.
fn str_of(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(obj.str()?.to_cow()?.into_owned())
}

/// `Spec._format_default`: fast path for formatting with DEFAULT_FORMAT and no color.
pub(crate) fn format_default(slf: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut parts: Vec<String> = Vec::new();

    let name = spec_name(slf)?;
    let namespace = spec_namespace(slf)?;
    if !name.is_empty() {
        if truthy_str(&namespace) && !is_concrete(slf)? {
            parts.push(format!("{}.{}", namespace.clone().unwrap(), name));
        } else {
            parts.push(name.clone());
        }
    }

    let versions = spec_versions(slf)?;
    if versions.is_truthy()? {
        let version_str = str_of(&versions)?;
        if !version_str.is_empty() && version_str != ":" {
            // only include if not full range
            parts.push(format!("@{version_str}"));
        }
    }

    let compiler_flags_str = str_of(&spec_compiler_flags(slf)?)?;
    if !compiler_flags_str.is_empty() {
        parts.push(compiler_flags_str);
    }

    let variants_str = str_of(&spec_variants(slf)?)?;
    if !variants_str.is_empty() {
        parts.push(variants_str);
    }

    if name.is_empty() && truthy_str(&namespace) {
        parts.push(format!(" namespace={}", namespace.unwrap()));
    }

    if let Some(architecture) = spec_architecture(slf)? {
        if architecture.is_truthy()? {
            for attr in ["platform", "os", "target"] {
                let value = architecture.getattr(attr)?;
                if value.is_truthy()? {
                    parts.push(format!(" {attr}={}", str_of(&value)?));
                }
            }
        }
    }

    // The blank is required for round-tripping: ``key=value /abc`` parses as variant and
    // abstract hash, whereas ``key=value/abc`` is parsed as a variant with value
    // ``value/abc``.
    let abstract_hash = slf.downcast::<Spec>()?.borrow().abstract_hash.clone();
    if truthy_str(&abstract_hash) {
        parts.push(format!(" /{}", abstract_hash.unwrap()));
    }

    Ok(parts.concat().trim().to_string())
}

/// `_anonymous_star`: whether a spec needs a star to disambiguate it from an anonymous
/// spec with variants.
fn anonymous_star(dep: &Bound<'_, PyAny>, dep_format: &str) -> PyResult<&'static str> {
    let py = dep.py();
    let child = edge_child(dep)?;

    // named spec never needs star
    if !spec_name(&child)?.is_empty() {
        return Ok("");
    }

    // virtuals without a name always need *: %c=* @4.0 foo=bar
    if !edge_virtuals(dep)?.is_empty() {
        return Ok("*");
    }

    // versions are first so checking for @ is faster than != VersionList(':')
    if dep_format.starts_with('@') {
        return Ok("");
    }

    // compiler flags are key-value pairs and can be ambiguous with virtual assignment
    if spec_compiler_flags(&child)?.is_truthy()? {
        return Ok("*");
    }

    // booleans come first, and they don't need a star; key-value pairs do
    let bool_member =
        registered(py, &registry::VARIANT_TYPE, "the VariantType enum")?.getattr("BOOL")?;
    let mut any_bool = false;
    for value in spec_variants(&child)?.call_method0("values")?.try_iter()? {
        if value?.getattr("type")?.eq(&bool_member)? {
            any_bool = true;
            break;
        }
    }
    if !any_bool {
        return Ok("*");
    }

    match spec_architecture(&child)? {
        Some(architecture) if architecture.is_truthy()? => Ok("*"),
        _ => Ok(""),
    }
}

/// `Spec._format_edge_attributes`.
pub(crate) fn format_edge_attributes(
    dep: &Bound<'_, PyAny>,
    deptypes: bool,
    virtuals: bool,
) -> PyResult<String> {
    let py = dep.py();
    let depflag = edge_depflag(dep)?;
    let deptypes_str = if deptypes && depflag != 0 {
        format!("deptypes={}", flag_to_tuple(depflag).join(","))
    } else {
        String::new()
    };
    let when = edge_when(dep)?;
    let when_str = if when.ne(&empty_spec_obj(py)?)? {
        format!("when='{}'", str_of(&when)?)
    } else {
        String::new()
    };
    let edge_vs = edge_virtuals(dep)?;
    let virtuals_str = if virtuals && !edge_vs.is_empty() {
        format!("virtuals={}", edge_vs.join(","))
    } else {
        String::new()
    };

    let attrs: Vec<&str> = [
        when_str.as_str(),
        deptypes_str.as_str(),
        virtuals_str.as_str(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    if attrs.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("[{}] ", attrs.join(" ")))
    }
}

/// The tri-state `color` argument as the Python object `format()` receives.
fn color_obj(py: Python<'_>, color: Option<bool>) -> Bound<'_, PyAny> {
    match color {
        Some(value) => pyo3::types::PyBool::new(py, value).to_owned().into_any(),
        None => py.None().into_bound(py),
    }
}

/// Stable sort by child name, like the reference `sorted(..., key=lambda x: x.spec.name)`.
fn sort_by_name<'py>(edges: Vec<Bound<'py, PyAny>>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut keyed = edges
        .into_iter()
        .map(|e| Ok((spec_name(&edge_child(&e)?)?, e)))
        .collect::<PyResult<Vec<_>>>()?;
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(keyed.into_iter().map(|(_, e)| e).collect())
}

/// `Spec._format_dependencies`: helper for formatting dependencies on specs.
pub(crate) fn format_dependencies(
    slf: &Bound<'_, PyAny>,
    format_string: Option<&Bound<'_, PyAny>>,
    include: Option<&Bound<'_, PyAny>>,
    deptypes: bool,
    color: Option<bool>,
    force_direct: bool,
) -> PyResult<String> {
    let py = slf.py();
    let format_string: Bound<'_, PyAny> = match format_string {
        Some(s) => s.clone(),
        None => registered(py, &registry::DEFAULT_FORMAT, "DEFAULT_FORMAT")?,
    };
    let include_pred = |edge: &Bound<'_, PyAny>| -> PyResult<bool> {
        match include {
            Some(f) if f.is_truthy()? => f.call1((edge,))?.is_truthy(),
            _ => Ok(true),
        }
    };

    let (direct, transitive): (Vec<Bound<'_, PyAny>>, Vec<Bound<'_, PyAny>>) = if is_concrete(slf)?
    {
        (out_edges(slf)?, Vec::new())
    } else {
        // lang.stable_partition on edge.direct
        let mut direct = Vec::new();
        let mut transitive = Vec::new();
        for edge in out_edges(slf)? {
            if edge_direct(&edge)? {
                direct.push(edge);
            } else {
                transitive.push(edge);
            }
        }
        (direct, transitive)
    };

    // helper for direct and transitive loops below
    let format_edge = |edge: &Bound<'_, PyAny>,
                       sigil: &str,
                       dep_spec: Option<&Bound<'_, PyAny>>|
     -> PyResult<String> {
        let dep_spec: Bound<'_, PyAny> = match dep_spec {
            Some(spec) if spec.is_truthy()? => spec.clone(),
            _ => edge_child(edge)?,
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item("color", color_obj(py, color))?;
        let dep_format: String = dep_spec
            .call_method("format", (&format_string,), Some(&kwargs))?
            .extract()?;

        let edge_attributes =
            if edge_depflag(edge)? != 0 || edge_when(edge)?.ne(&empty_spec_obj(py)?)? {
                format_edge_attributes(edge, deptypes, false)?
            } else {
                String::new()
            };
        let edge_vs = edge_virtuals(edge)?;
        let virtuals = if edge_vs.is_empty() {
            String::new()
        } else {
            format!("{}=", edge_vs.join(","))
        };
        let star = anonymous_star(edge, &dep_format)?;

        Ok(format!(
            "{sigil}{edge_attributes}{star}{virtuals}{dep_format}"
        ))
    };

    let mut parts: Vec<String> = Vec::new();

    // direct dependencies
    let aliases = registered(
        py,
        &registry::LEGACY_COMPILER_ALIASES,
        "BUILTIN_TO_LEGACY_COMPILER",
    )?;
    for edge in sort_by_name(direct)? {
        if !include_pred(&edge)? {
            continue;
        }

        // replace legacy compiler names
        let child = edge_child(&edge)?;
        let old_name = child.getattr("name")?;
        let new_name = aliases.call_method1("get", (&old_name,))?;
        // this is ugly but copies can be expensive: temporarily rename the node, and
        // restore the name like the reference `finally` regardless of the outcome
        let formatted = (|| -> PyResult<String> {
            let mut sigil = "%";
            if new_name.is_truthy()? {
                child.setattr("name", &new_name)?;
            }

            if edge_propagation(&edge)? == PROPAGATION_PREFERENCE {
                sigil = "%%";
            }

            format_edge(&edge, sigil, Some(&child))
        })();
        let restored = child.setattr("name", &old_name);
        restored?;
        parts.push(formatted?);
    }

    if is_concrete(slf)? {
        // Concrete specs should go no further, as the complexity below is O(paths)
        return Ok(parts.join(" ").trim().to_string());
    }

    // transitive dependencies (with any direct dependencies)
    for edge in sort_by_name(transitive)? {
        if !include_pred(&edge)? {
            continue;
        }
        let sigil = if force_direct { "%" } else { "^" }; // hack til direct deps repr. better
        let child = edge_child(&edge)?;
        parts.push(format_edge(&edge, sigil, Some(&child))?);

        // also recursively add any direct dependencies of transitive dependencies
        if has_deps(&child)? {
            let kwargs = PyDict::new(py);
            kwargs.set_item("format_string", &format_string)?;
            kwargs.set_item("include", include)?;
            kwargs.set_item("deptypes", deptypes)?;
            kwargs.set_item("_force_direct", force_direct)?;
            let nested: String = child
                .call_method("_format_dependencies", (), Some(&kwargs))?
                .extract()?;
            parts.push(nested);
        }
    }

    Ok(parts.join(" ").trim().to_string())
}

/// `Spec._long_spec`: helper for `long_spec` and `clong_spec`.
pub(crate) fn long_spec(slf: &Bound<'_, PyAny>, color: Option<bool>) -> PyResult<String> {
    let py = slf.py();
    if is_concrete(slf)? {
        let kwargs = PyDict::new(py);
        kwargs.set_item(
            "format",
            registered(py, &registry::DISPLAY_FORMAT, "DISPLAY_FORMAT")?,
        )?;
        kwargs.set_item("color", color_obj(py, color))?;
        return slf.call_method("tree", (), Some(&kwargs))?.extract();
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("color", color_obj(py, color))?;
    let head: String = slf.call_method("format", (), Some(&kwargs))?.extract()?;
    let deps: String = slf
        .call_method("_format_dependencies", (), Some(&kwargs))?
        .extract()?;
    Ok(format!("{head} {deps}").trim().to_string())
}

/// `Spec._str`: string representation of this spec.
pub(crate) fn str_impl(slf: &Bound<'_, PyAny>, color: Option<bool>) -> PyResult<String> {
    let py = slf.py();
    if is_concrete(slf)? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("color", color_obj(py, color))?;
        return slf
            .call_method(
                "format",
                (PyString::new(py, "{name}{@version}{/hash}"),),
                Some(&kwargs),
            )?
            .extract();
    }

    if !has_deps(slf)? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("color", color_obj(py, color))?;
        return slf.call_method("format", (), Some(&kwargs))?.extract();
    }

    long_spec(slf, color)
}
