// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of the ASP fact-string building in `spack.solver.core` / `asp.py`:
//! `AspFunction.__str__` (with its exact escaping and quoting rules) and the
//! problem-instance line buffer (`fact` / `append` / `title` / strip + sort).
//!
//! Python -> Rust mapping:
//! - `AspFunction(name, args)` -> [`AspFun`]; `__str__` -> [`AspFun::render`]
//! - `AspVar` -> [`AspArg::Var`] (renders as its bare name)
//! - `ProblemInstanceBuilder.fact/append/title/newline` -> [`ProblemBuffer`]
//! - `_strip_asp_problem` + `problem.sort()` -> [`ProblemBuffer::stripped_sorted`]

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AspArg {
    /// `type(arg) is str` (and str subclasses): escaped and double-quoted.
    Str(String),
    /// `int` but not `bool`: bare.
    Int(i64),
    /// `AspVar`: bare name.
    Var(String),
    /// Nested `AspFunction`.
    Fun(AspFun),
    /// The fallback branch: `f'"{arg}"'` — str() of the object, quoted, NOT escaped.
    /// Covers bool ("True"/"False") and any other object.
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AspFun {
    pub name: String,
    pub args: Vec<AspArg>,
}

/// `arg.replace("\\", r"\\").replace("\n", r"\n").replace('"', r"\"")`
fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
}

impl AspFun {
    pub fn render_into(&self, out: &mut String) {
        out.push_str(&self.name);
        out.push('(');
        for (i, arg) in self.args.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match arg {
                AspArg::Str(s) => {
                    out.push('"');
                    escape_into(out, s);
                    out.push('"');
                }
                AspArg::Int(v) => {
                    let _ = write!(out, "{}", v);
                }
                AspArg::Var(name) => out.push_str(name),
                AspArg::Fun(f) => f.render_into(out),
                AspArg::Raw(s) => {
                    out.push('"');
                    out.push_str(s);
                    out.push('"');
                }
            }
        }
        out.push(')');
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(32);
        self.render_into(&mut out);
        out
    }
}

/// The problem instance as a list of lines, so the unsorted debug output and the
/// benchmarking shuffle path keep their Python behavior.
#[derive(Debug, Default)]
pub struct ProblemBuffer {
    pub lines: Vec<String>,
}

impl ProblemBuffer {
    pub fn new() -> Self {
        ProblemBuffer { lines: Vec::new() }
    }

    /// `ProblemInstanceBuilder.fact`: the rendered atom plus a trailing period.
    pub fn fact(&mut self, atom: &AspFun) {
        let mut line = String::with_capacity(48);
        atom.render_into(&mut line);
        line.push('.');
        self.lines.push(line);
    }

    pub fn append(&mut self, rule: String) {
        self.lines.push(rule);
    }

    /// `ProblemInstanceBuilder.title`: a newline then `%` + 76 repeats of `char`,
    /// `% title`, `%` + separator.
    pub fn title(&mut self, header: &str, ch: char) {
        let sep: String = std::iter::repeat(ch).take(76).collect();
        self.newline();
        self.lines.push(format!("%{}", sep));
        self.lines.push(format!("% {}", header));
        self.lines.push(format!("%{}", sep));
    }

    pub fn newline(&mut self) {
        self.lines.push(String::new());
    }

    /// `_strip_asp_problem` + `problem.sort()`: drop empty lines, sort the rest.
    pub fn stripped_sorted(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .lines
            .iter()
            .filter(|l| !l.is_empty())
            .cloned()
            .collect();
        lines.sort_unstable();
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fun(name: &str, args: Vec<AspArg>) -> AspFun {
        AspFun {
            name: name.to_string(),
            args,
        }
    }

    #[test]
    fn renders_like_python() {
        assert_eq!(
            fun(
                "attr",
                vec![
                    AspArg::Str("version".into()),
                    AspArg::Str("a\"b\\c\nd".into()),
                    AspArg::Int(-3),
                    AspArg::Var("X".into()),
                    AspArg::Raw("True".into()),
                    AspArg::Fun(fun(
                        "node",
                        vec![AspArg::Int(0), AspArg::Str("zlib".into())]
                    )),
                ]
            )
            .render(),
            "attr(\"version\",\"a\\\"b\\\\c\\nd\",-3,X,\"True\",node(0,\"zlib\"))"
        );
        assert_eq!(fun("empty", vec![]).render(), "empty()");
    }

    #[test]
    fn buffer_fact_title_strip_sort() {
        let mut buf = ProblemBuffer::new();
        buf.title("Header", '-');
        buf.fact(&fun("b", vec![]));
        buf.fact(&fun("a", vec![]));
        assert_eq!(buf.lines[0], "");
        assert_eq!(buf.lines[1], format!("%{}", "-".repeat(76)));
        assert_eq!(buf.lines[2], "% Header");
        let sorted = buf.stripped_sorted();
        assert_eq!(
            sorted[sorted.len() - 2..],
            ["a().".to_string(), "b().".to_string()]
        );
    }
}
