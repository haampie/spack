// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! `Spec("...")` in rust mode: replays the event stream of
//! `spack_spec_core::parse::parse_events_one` into a Spec DAG through the same construction
//! calls the Python `SpecParser` makes, with the exact `parse_one_or_raise` semantics (single
//! spec, trailing-text rejection, error classes and underlines). The specs under construction
//! are always fresh and mutable, so node state is written straight into the Rust struct;
//! everything that needs Python (spec files, error classes with colorized underlines) goes
//! through callables registered by `spack.spec`.

use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString, PyTuple};
use pyo3::IntoPyObjectExt;

use spack_spec_core::parse::events::{
    legacy_compiler_to_builtin, Propagation, Span, SpecParseError,
};
use spack_spec_core::parse::{parse_events_one, ParsingErrorKind, SpecAction, SpecEvent};

use crate::algebra::{is_spec_error, registered, DT_ALL};
use crate::registry;

/// An exception built by a registered Python factory, as a raisable `PyErr`.
fn factory_error<'py>(
    py: Python<'py>,
    cell: &'static pyo3::sync::PyOnceLock<Py<PyAny>>,
    what: &str,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> PyErr {
    match registered(py, cell, what).and_then(|f| f.call1(args)) {
        Ok(exc) => PyErr::from_value(exc),
        Err(e) => e,
    }
}

/// `SpecParsingError(message, token, text)` via the registered factory; `span` of `None`
/// mirrors Python passing a `None` token (no underline).
fn parsing_error(py: Python<'_>, message: &str, span: Option<Span>, text: &str) -> PyErr {
    factory_error(
        py,
        &registry::PARSING_ERROR_FACTORY,
        "the SpecParsingError factory",
        (message, span.map(|s| s.start), span.map(|s| s.end), text),
    )
}

/// A core parse error as the Python exception `SpecParser` raises for the same input.
fn core_error_to_pyerr(py: Python<'_>, err: SpecParseError<'_>, text: &str) -> PyErr {
    match err {
        SpecParseError::Tokenization(_) => factory_error(
            py,
            &registry::TOKENIZATION_ERROR_FACTORY,
            "the SpecTokenizationError factory",
            (text,),
        ),
        // PY: `spack.deptypes.canonicalize` raises a bare ValueError, never wrapped.
        SpecParseError::Parsing(p) if p.kind == ParsingErrorKind::InvalidDeptype => {
            PyValueError::new_err(p.message)
        }
        SpecParseError::Parsing(p) => {
            let span = p.token.map(|t| Span {
                start: t.start,
                end: t.end,
            });
            parsing_error(py, &p.message, span, text)
        }
    }
}

/// `raise SpecParsingError(str(e), token, text) from e`.
fn wrap_in_parsing_error(py: Python<'_>, err: PyErr, span: Span, text: &str) -> PyErr {
    let message = match err.value(py).str() {
        Ok(s) => s.to_string_lossy().into_owned(),
        Err(e) => return e,
    };
    let wrapped = parsing_error(py, &message, Some(span), text);
    wrapped.set_cause(py, Some(err));
    wrapped
}

/// The characters of `text` in the (char-indexed) span, like Python `text[start:end]`.
fn char_slice(text: &str, span: Span) -> String {
    text.chars()
        .skip(span.start)
        .take(span.end.saturating_sub(span.start))
        .collect()
}

/// A fresh node through the registered Python Spec class, like the parser's `Spec()`.
fn new_spec(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    registered(py, &registry::SPEC_CLASS, "the Spec class")?.call0()
}

/// The edge properties accumulated between a `^`/`%` sigil and the completed node, i.e. the
/// Python `edge_properties` dict.
struct EdgeProps<'py, 'a> {
    depflag: u64,
    virtuals: Vec<&'a str>,
    direct: bool,
    propagation: Propagation,
    when: Option<Bound<'py, PyAny>>,
    /// Span of the node's name token, to recover the pre-alias spelling for the
    /// dependent-of-concrete-root error message.
    name_span: Option<Span>,
}

/// The state of `SpecParser.next_spec` while replaying one spec's events: the root, the node
/// direct edges attach to, the unattached `^` dependency, and the edge whose node is open.
struct Replay<'py, 'a> {
    py: Python<'py>,
    text: &'a str,
    root: Bound<'py, PyAny>,
    current: Bound<'py, PyAny>,
    pending: Option<(Bound<'py, PyAny>, EdgeProps<'py, 'a>, Span)>,
    open: Option<(Bound<'py, PyAny>, EdgeProps<'py, 'a>)>,
    /// Span of the last replayed event: `ctx.current_token` at the matching Python point.
    last_span: Span,
    saw_when: bool,
}

impl<'py, 'a> Replay<'py, 'a> {
    fn new(py: Python<'py>, text: &'a str, root: Bound<'py, PyAny>) -> Self {
        Replay {
            py,
            text,
            current: root.clone(),
            root,
            pending: None,
            open: None,
            last_span: Span { start: 0, end: 0 },
            saw_when: false,
        }
    }

    /// The node currently receiving node-attribute events.
    fn node(&self) -> &Bound<'py, PyAny> {
        match &self.open {
            Some((node, _)) => node,
            None => &self.root,
        }
    }

    /// Replay the events in token order. On a partial stream (core parse error downstream)
    /// this is the whole replay: like Python, the end-of-spec attachments never run, while the
    /// edges whose node completed before the error have already attached.
    fn replay(&mut self, events: &[SpecEvent<'a>]) -> PyResult<()> {
        for event in events {
            match &event.action {
                SpecAction::StartRootNode => continue, // one spec: the leading event only
                SpecAction::StartDependencyNode {
                    direct,
                    propagation,
                    virtuals,
                } => self.open_edge(0, virtuals.clone(), *direct, *propagation)?,
                SpecAction::StartEdgeProperties {
                    direct,
                    propagation,
                } => self.open_edge(0, Vec::new(), *direct, *propagation)?,
                SpecAction::EndDependencyNode => self.finish_open_edge(event.span)?,
                SpecAction::EdgeDeptypes { depflag } => {
                    self.props()?.depflag = u64::from(*depflag);
                }
                SpecAction::EdgeWhen { when } => {
                    // PY: `attributes["when"] = parse_one_or_raise(attributes["when"])`.
                    let spec = parse_when(self.py, when)?;
                    self.saw_when = true;
                    self.props()?.when = Some(spec);
                }
                SpecAction::EndEdgeProperties { virtuals } => {
                    self.props()?.virtuals = virtuals.clone();
                }
                SpecAction::NodeName { name, namespace } => {
                    {
                        let node = self.node().clone();
                        let cell = node.downcast::<crate::spec::Spec>()?;
                        let mut this = cell.borrow_mut();
                        this.name = (*name).to_string();
                        if let Some(namespace) = namespace {
                            this.namespace = Some((*namespace).to_string());
                        }
                    }
                    if let Some((_, props)) = self.open.as_mut() {
                        props.name_span = Some(event.span);
                    }
                }
                SpecAction::Star => {} // anonymous: the name stays unset
                SpecAction::NodeVersion { version } => self.set_version(version)?,
                SpecAction::BoolVariant {
                    name,
                    value,
                    propagate,
                } => {
                    let value = value.into_bound_py_any(self.py)?;
                    self.add_flag(name, &value, *propagate, true, event.span)?;
                }
                SpecAction::KeyValuePair {
                    name,
                    value,
                    propagate,
                    concrete,
                } => {
                    let value = PyString::new(self.py, value).into_any();
                    self.add_flag(name, &value, *propagate, *concrete, event.span)?;
                }
                SpecAction::DagHash { hash } => {
                    let node = self.node().clone();
                    node.downcast::<crate::spec::Spec>()?
                        .borrow_mut()
                        .abstract_hash = Some((*hash).to_string());
                }
                SpecAction::Filename { path } => {
                    // Spec files stay Python: `FileParser.parse` via the registered callable.
                    let parse_file =
                        registered(self.py, &registry::SPEC_FILE_PARSER, "the spec file parser")?;
                    parse_file.call1((path, self.node()))?;
                }
            }
            self.last_span = event.span;
        }
        Ok(())
    }

    /// The end of `next_spec`: attach the pending `^` dependency, then canonicalize conditional
    /// edges.
    fn finish(&mut self) -> PyResult<()> {
        self.attach_pending()?;

        // PY: only specs with a when-conditioned edge pay for the canonicalization pass,
        // on the root first and then on each of its dependencies.
        if self.saw_when {
            if let Err(e) = self.canonicalize() {
                if is_spec_error(self.py, &e)? {
                    return Err(wrap_in_parsing_error(self.py, e, self.last_span, self.text));
                }
                return Err(e);
            }
        }
        Ok(())
    }

    fn open_edge(
        &mut self,
        depflag: u64,
        virtuals: Vec<&'a str>,
        direct: bool,
        propagation: Propagation,
    ) -> PyResult<()> {
        let node = new_spec(self.py)?;
        self.open = Some((
            node,
            EdgeProps {
                depflag,
                virtuals,
                direct,
                propagation,
                when: None,
                name_span: None,
            },
        ));
        Ok(())
    }

    fn props(&mut self) -> PyResult<&mut EdgeProps<'py, 'a>> {
        match &mut self.open {
            Some((_, props)) => Ok(props),
            None => Err(PyValueError::new_err(
                "edge attribute event outside an open edge",
            )),
        }
    }

    /// The node of the open edge is complete: the checks and attachments Python performs
    /// between `_parse_node` returning and the loop pulling its next token. `span` is the
    /// node's last token, which is what an error here underlines.
    fn finish_open_edge(&mut self, span: Span) -> PyResult<()> {
        let Some((node, props)) = self.open.take() else {
            return Ok(());
        };
        // PY: `_parse_node` refuses dependents of a concrete (file-loaded) root. Python
        // checks before rewriting legacy compiler aliases, so the message shows the
        // pre-alias spelling; undo the rewrite the `NodeName` event already applied.
        if crate::algebra::is_concrete(&self.root)? {
            let current = crate::algebra::spec_name(&node)?;
            let original = props.name_span.map(|span| {
                let slice: String = char_slice(self.text, span);
                slice.rsplit('.').next().unwrap_or_default().to_string()
            });
            let aliased = original.as_deref().is_some_and(|orig| {
                orig != current && legacy_compiler_to_builtin(orig) == Some(current.as_str())
            });
            if aliased {
                node.downcast::<crate::spec::Spec>()?.borrow_mut().name = original.unwrap();
            }
            let node_str = node.str()?.to_string();
            if aliased {
                node.downcast::<crate::spec::Spec>()?.borrow_mut().name = current;
            }
            let cls = registered(self.py, &registry::SPEC_ERROR, "SpecError")?;
            let exc = cls.call1((self.root.str()?, format!("^{node_str}")))?;
            return Err(PyErr::from_value(exc));
        }
        if props.direct {
            let current = self.current.clone();
            self.attach(&current, &node, &props, span)
        } else {
            self.attach_pending()?;
            self.current = node.clone();
            self.pending = Some((node, props, span));
            Ok(())
        }
    }

    /// `SpecParser._attach_pending`: the pending `^` dependency's sub-dag is complete now.
    fn attach_pending(&mut self) -> PyResult<()> {
        if let Some((node, props, span)) = self.pending.take() {
            let root = self.root.clone();
            self.attach(&root, &node, &props, span)?;
        }
        Ok(())
    }

    /// `SpecParser._attach_dependency`: `_add_dependency` with `SpecError` wrapped into a
    /// `SpecParsingError` at the token the edge finished on.
    fn attach(
        &self,
        parent: &Bound<'py, PyAny>,
        node: &Bound<'py, PyAny>,
        props: &EdgeProps<'py, 'a>,
        span: Span,
    ) -> PyResult<()> {
        let py = self.py;
        let virtuals = PyTuple::new(py, &props.virtuals)?;
        let policy = registered(py, &registry::PROPAGATION_POLICY, "PropagationPolicy")?.call1(
            (match props.propagation {
                Propagation::None => crate::algebra::PROPAGATION_NONE,
                Propagation::Preference => crate::algebra::PROPAGATION_PREFERENCE,
            },),
        )?;
        let result = crate::graph::add_dependency_edge(
            parent,
            node,
            props.depflag,
            virtuals.as_any(),
            props.direct,
            Some(&policy),
            props.when.as_ref(),
        );
        match result {
            Ok(()) => Ok(()),
            Err(e) if is_spec_error(py, &e)? => Err(wrap_in_parsing_error(py, e, span, self.text)),
            Err(e) => Err(e),
        }
    }

    /// The node-parser `add_flag` wrapper: any `Exception` becomes a `SpecParsingError` with
    /// the flag token underlined.
    fn add_flag(
        &self,
        name: &str,
        value: &Bound<'py, PyAny>,
        propagate: bool,
        concrete: bool,
        span: Span,
    ) -> PyResult<()> {
        match crate::graph::add_flag(self.node(), name, value, propagate, concrete) {
            Ok(()) => Ok(()),
            Err(e) if e.is_instance_of::<PyException>(self.py) => {
                Err(wrap_in_parsing_error(self.py, e, span, self.text))
            }
            Err(e) => Err(e),
        }
    }

    /// `initial_spec.versions = VersionList([from_string(...)])` plus the git lookup.
    fn set_version(&self, version: &str) -> PyResult<()> {
        let py = self.py;
        let parsed = crate::version::from_string(py, version)?;
        let items = PyList::new(py, [parsed])?;
        let versions = py
            .get_type::<crate::version::VersionList>()
            .call1((items,))?;
        let has_git = versions
            .try_iter()?
            .any(|v| v.is_ok_and(|v| v.is_instance_of::<crate::version::GitVersion>()));
        let node = self.node().clone();
        node.downcast::<crate::spec::Spec>()?.borrow_mut().versions = Some(versions.unbind());
        // The lookup machinery is Python; skip the call when no GitVersion needs it.
        if has_git {
            node.call_method0("attach_git_version_lookup")?;
        }
        Ok(())
    }

    /// The `saw_when` canonicalization pass at the end of `next_spec`.
    fn canonicalize(&self) -> PyResult<()> {
        crate::algebra::canonicalize_conditional_edges(&self.root)?;
        let edges = crate::graph::select_edges(
            &self.root.getattr("_dependencies")?,
            None,
            None,
            DT_ALL,
            None,
        )?;
        for edge in edges {
            crate::algebra::canonicalize_conditional_edges(&edge.getattr("spec")?)?;
        }
        Ok(())
    }
}

/// `parse_one_or_raise(text, initial_spec)` up to (and excluding) toolchain expansion, which
/// `Spec.__init__` never requests. Returns `None` exactly when Python would: empty input and
/// no `initial_spec`.
fn parse_one_impl<'py>(
    py: Python<'py>,
    text: &str,
    initial_spec: Option<&Bound<'py, PyAny>>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let (events, outcome) = parse_events_one(text);
    if events.is_empty() {
        return match outcome {
            Err(e) => Err(core_error_to_pyerr(py, e, text)),
            Ok(_) => Ok(initial_spec.cloned()), // no tokens: the untouched initial spec
        };
    }
    let spec = match initial_spec {
        Some(spec) => spec.clone(),
        None => new_spec(py)?,
    };
    let mut replay = Replay::new(py, text, spec.clone());
    replay.replay(&events)?;
    let leftover = match outcome {
        // A binding-level error during the partial replay above wins, like the Python
        // parser's interleaving of Spec mutations with parsing; the core error comes next.
        Err(e) => return Err(core_error_to_pyerr(py, e, text)),
        Ok(leftover) => leftover,
    };
    replay.finish()?;
    if let Some(token) = leftover {
        return Err(factory_error(
            py,
            &registry::MORE_SPECS_ERROR_FACTORY,
            "the trailing-spec error factory",
            (text, token.start, token.value.chars().count()),
        ));
    }
    Ok(Some(spec))
}

/// A `when=` edge attribute: `parse_one_or_raise(value)` on the inner spec string.
fn parse_when<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    parse_one_or_raise(py, text, None)
}

/// `spack.spec_parser.parse_one_or_raise` minus toolchain expansion: exactly one spec from
/// `text`, parsed into `initial_spec` when given and into a fresh Spec otherwise. Called by
/// `Spec.__init__` on the string path and by the module-level rebind in `spack.spec_parser`.
#[pyfunction]
#[pyo3(signature = (text, initial_spec=None))]
pub fn parse_one_or_raise<'py>(
    py: Python<'py>,
    text: &str,
    initial_spec: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    match parse_one_impl(py, text, initial_spec)? {
        Some(spec) => Ok(spec),
        // PY: empty input and no buffer to parse into; the trailing-text check comes first.
        None => Err(PyValueError::new_err(
            "expected a single spec, but got none",
        )),
    }
}
