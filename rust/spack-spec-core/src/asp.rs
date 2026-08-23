// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Port of the ASP fact-string building in `spack.solver.core` / `asp.py`:
//! `AspFunction.__str__` (with its exact escaping and quoting rules) and the
//! problem-instance line buffer (`fact` / `append` / `title` / strip + sort).
//!
//! Terms are written straight into a reusable buffer instead of being built as a tree first:
//! a binding walks the objects it was handed and pushes each argument as it goes, so rendering
//! a problem instance copies each argument's text exactly once, into the output.
//!
//! Python -> Rust mapping:
//! - `AspFunction.__str__` -> [`AspWriter`] (`open` / `*_arg` / `close`)
//! - `AspVar` -> [`AspWriter::bare_arg`] (renders as its bare name)
//! - `ProblemInstanceBuilder.fact/append/title/newline` -> [`ProblemBuffer`]
//! - `_strip_asp_problem` + `problem.sort()` + `"\n".join(...)` ->
//!   [`ProblemBuffer::stripped_sorted_joined`]

use std::fmt::Write as _;

/// Writes ASP terms into a buffer: `open` a function, push its arguments, `close` it. Nested
/// functions are just an `open`/`close` pair inside another, so one writer renders a whole
/// term without allocating anything per argument.
#[derive(Debug, Default)]
pub struct AspWriter {
    out: String,
    /// Whether the function at each open level already has an argument, so the next one is
    /// preceded by a comma. The last entry is the level being written.
    populated: Vec<bool>,
}

impl AspWriter {
    pub fn new() -> Self {
        AspWriter {
            out: String::with_capacity(64),
            populated: Vec::new(),
        }
    }

    /// The comma before an argument, and the bookkeeping that says the next one needs one.
    fn separate(&mut self) {
        if let Some(populated) = self.populated.last_mut() {
            if *populated {
                self.out.push(',');
            }
            *populated = true;
        }
    }

    /// Begin `name(`, as an argument of the function currently open, if any.
    pub fn open(&mut self, name: &str) {
        self.separate();
        self.out.push_str(name);
        self.out.push('(');
        self.populated.push(false);
    }

    pub fn close(&mut self) {
        self.out.push(')');
        self.populated.pop();
    }

    /// A `str` argument: escaped and double-quoted, like Python's
    /// `arg.replace("\\", r"\\").replace("\n", r"\n").replace('"', r"\"")`.
    pub fn str_arg(&mut self, s: &str) {
        self.separate();
        self.out.push('"');
        for c in s.chars() {
            match c {
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '"' => self.out.push_str("\\\""),
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }

    /// An `int` argument, bare.
    pub fn int_arg(&mut self, value: i64) {
        self.separate();
        let _ = write!(self.out, "{}", value);
    }

    /// An argument that renders as itself: an `AspVar`'s name, or an integer too large to have
    /// come through [`Self::int_arg`].
    pub fn bare_arg(&mut self, s: &str) {
        self.separate();
        self.out.push_str(s);
    }

    /// The fallback branch, `f'"{arg}"'`: the object's `str()`, quoted but NOT escaped. Covers
    /// `bool` ("True"/"False") and anything else.
    pub fn raw_arg(&mut self, s: &str) {
        self.separate();
        self.out.push('"');
        self.out.push_str(s);
        self.out.push('"');
    }

    /// The term written so far. Every `open` must have been closed.
    pub fn as_str(&self) -> &str {
        debug_assert!(self.populated.is_empty(), "an open function was not closed");
        &self.out
    }

    /// Drop the term and keep the allocation for the next one.
    pub fn clear(&mut self) {
        self.out.clear();
        self.populated.clear();
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

    /// `ProblemInstanceBuilder.fact`: a rendered atom plus a trailing period.
    pub fn fact(&mut self, atom: &str) {
        let mut line = String::with_capacity(atom.len() + 1);
        line.push_str(atom);
        line.push('.');
        self.lines.push(line);
    }

    pub fn append(&mut self, rule: String) {
        self.lines.push(rule);
    }

    /// `ProblemInstanceBuilder.title`: a newline then `%` + 76 repeats of `fill`,
    /// `% title`, `%` + separator. Python multiplies a string, so a multi-character
    /// `fill` repeats whole.
    pub fn title(&mut self, header: &str, fill: &str) {
        let sep: String = fill.repeat(76);
        self.newline();
        self.lines.push(format!("%{}", sep));
        self.lines.push(format!("% {}", header));
        self.lines.push(format!("%{}", sep));
    }

    pub fn newline(&mut self) {
        self.lines.push(String::new());
    }

    /// `_strip_asp_problem` + `problem.sort()`: drop empty lines, sort the rest. The lines are
    /// borrowed; a problem instance is hundreds of thousands of them and copying them to sort
    /// would cost more than the sort. Rust orders `str` by UTF-8 bytes, which is the code point
    /// order Python sorts by.
    pub fn stripped_sorted(&self) -> Vec<&str> {
        let mut lines: Vec<&str> = self
            .lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(String::as_str)
            .collect();
        lines.sort_unstable();
        lines
    }

    /// The instance clingo is handed: [`Self::stripped_sorted`] joined with newlines.
    pub fn stripped_sorted_joined(&self) -> String {
        let lines = self.stripped_sorted();
        let size = lines.iter().map(|l| l.len() + 1).sum::<usize>();
        let mut out = String::with_capacity(size);
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_like_python() {
        let mut w = AspWriter::new();
        w.open("attr");
        w.str_arg("version");
        w.str_arg("a\"b\\c\nd");
        w.int_arg(-3);
        w.bare_arg("X");
        w.raw_arg("True");
        w.open("node");
        w.int_arg(0);
        w.str_arg("zlib");
        w.close();
        w.close();
        assert_eq!(
            w.as_str(),
            "attr(\"version\",\"a\\\"b\\\\c\\nd\",-3,X,\"True\",node(0,\"zlib\"))"
        );

        w.clear();
        w.open("empty");
        w.close();
        assert_eq!(w.as_str(), "empty()");
    }

    /// A nested function is an argument like any other: it gets a comma before it, and the
    /// arguments after it get one too.
    #[test]
    fn nesting_separates_like_a_plain_argument() {
        let mut w = AspWriter::new();
        w.open("outer");
        w.open("first");
        w.close();
        w.open("second");
        w.str_arg("x");
        w.close();
        w.int_arg(1);
        w.close();
        assert_eq!(w.as_str(), "outer(first(),second(\"x\"),1)");
    }

    #[test]
    fn buffer_fact_title_strip_sort() {
        let mut buf = ProblemBuffer::new();
        buf.title("Header", "-");
        buf.fact("b()");
        buf.fact("a()");
        assert_eq!(buf.lines[0], "");
        assert_eq!(buf.lines[1], format!("%{}", "-".repeat(76)));
        assert_eq!(buf.lines[2], "% Header");
        let sorted = buf.stripped_sorted();
        assert_eq!(sorted[sorted.len() - 2..], ["a().", "b()."]);
        assert_eq!(
            buf.stripped_sorted_joined(),
            sorted.join("\n"),
            "the joined instance is the sorted lines"
        );
    }
}
