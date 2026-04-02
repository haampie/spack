#!/usr/bin/env python3
"""
Auto-fix TYPE_CHECKING candidates for ``spack.*`` imports.

For every top-level ``import spack.*`` / ``from spack.* import ...`` that is
only referenced in annotation positions, this script:

  1. Moves the import behind ``if TYPE_CHECKING:`` (adding the block if absent,
     adding ``from typing import TYPE_CHECKING`` if absent).
  2. Wraps any bare annotation that referenced the import in double-quotes so
     it is not evaluated at runtime (because Spack doesn't use
     ``from __future__ import annotations``).

Usage
-----
  python dev-utils/fix_type_only_imports.py [path] [--dry-run] [--json-report report.json]

  path           file or directory (default: lib/spack/spack/, vendor/ + test/ skipped)
  --dry-run      show diffs without writing files
  --json-report  write a per-file summary JSON to the given path
"""

from __future__ import annotations

import argparse
import ast
import difflib
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple

SKIP_DIRS = {"vendor", "test"}
DEFAULT_ROOT = Path(__file__).parent.parent / "lib" / "spack" / "spack"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _is_type_checking_guard(test: ast.expr) -> bool:
    if isinstance(test, ast.Name) and test.id == "TYPE_CHECKING":
        return True
    if (
        isinstance(test, ast.Attribute)
        and test.attr == "TYPE_CHECKING"
        and isinstance(test.value, ast.Name)
        and test.value.id == "typing"
    ):
        return True
    return False


def _dotted_chain(node: ast.expr) -> Optional[List[str]]:
    parts: List[str] = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
        return list(reversed(parts))
    return None


# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclass
class CandidateImport:
    """A top-level import that can be moved behind TYPE_CHECKING."""

    match_key: str  # dotted prefix used in source (e.g. "spack.spec")
    full_module: str  # for keying / dedup
    imported_name: Optional[str]  # for `from X import Y`, the symbol Y; else None
    stmt_node: ast.stmt  # the original Import / ImportFrom AST node
    # The import statement as it should appear inside the TYPE_CHECKING block.
    guarded_text: str


@dataclass
class TextEdit:
    """Replace source text from [start_line, start_col) to [end_line, end_col)."""

    start_line: int  # 1-based
    start_col: int  # 0-based
    end_line: int  # 1-based, inclusive
    end_col: int  # 0-based, exclusive
    replacement: str

    @property
    def sort_key(self) -> Tuple[int, int]:
        return (self.start_line, self.start_col)


# ---------------------------------------------------------------------------
# Step 1 – collect candidates  (same logic as check script, spack.* only)
# ---------------------------------------------------------------------------


def _make_candidate(node: ast.stmt) -> List[CandidateImport]:
    results: List[CandidateImport] = []
    if isinstance(node, ast.Import):
        for alias in node.names:
            if not alias.name.startswith("spack."):
                continue
            match_key = alias.asname if alias.asname else alias.name
            full_module = alias.name
            asname = f" as {alias.asname}" if alias.asname else ""
            guarded_text = f"import {alias.name}{asname}"
            results.append(CandidateImport(match_key, full_module, None, node, guarded_text))
    elif isinstance(node, ast.ImportFrom):
        module = node.module or ""
        if not module.startswith("spack.") and module != "spack":
            return results
        for alias in node.names:
            if alias.name == "*":
                continue
            match_key = alias.asname if alias.asname else alias.name
            full_module = f"{module}.{alias.name}" if module else alias.name
            asname = f" as {alias.asname}" if alias.asname else ""
            guarded_text = f"from {module} import {alias.name}{asname}"
            results.append(CandidateImport(match_key, full_module, alias.name, node, guarded_text))
    return results


def _node_name(node: ast.stmt) -> str:
    """Return a stable key for an import statement node (for deduplication)."""
    return ast.dump(node)


def collect_candidates(tree: ast.Module, source_lines: List[str]) -> Dict[str, CandidateImport]:
    """Return map full_module → CandidateImport for spack.* top-level imports."""
    # First pass: gather all top-level imports and guarded imports.
    top_level: List[CandidateImport] = []
    guarded_full_modules: Set[str] = set()

    for stmt in tree.body:
        if isinstance(stmt, (ast.Import, ast.ImportFrom)):
            top_level.extend(_make_candidate(stmt))
        elif isinstance(stmt, ast.If) and _is_type_checking_guard(stmt.test):
            for inner in stmt.body:
                if isinstance(inner, (ast.Import, ast.ImportFrom)):
                    for c in _make_candidate(inner):
                        guarded_full_modules.add(c.full_module)

    # Keep only those not already guarded.
    candidates: Dict[str, CandidateImport] = {}
    for c in top_level:
        if c.full_module not in guarded_full_modules:
            candidates[c.full_module] = c  # last binding wins (rare duplicates)

    return candidates


# ---------------------------------------------------------------------------
# Step 2 – classify usages to confirm they are annotation-only
# ---------------------------------------------------------------------------


class UsageClassifier(ast.NodeVisitor):
    """
    Visit every reference to a set of match_keys and record whether each
    usage is annotation-only or has runtime uses.
    """

    def __init__(self, match_key_to_full: Dict[str, str]) -> None:
        self._mk_to_full = match_key_to_full
        self._sorted_keys = sorted(match_key_to_full, key=lambda k: -len(k))
        # full_module → (has_runtime_use, annotation_nodes)
        self.runtime_use: Set[str] = set()
        # full_module → list of (annotation_node, is_already_string)
        # annotation_node is the TOP-LEVEL annotation expression (what to stringify)
        self.annotation_nodes: Dict[str, List[Tuple[ast.expr, bool]]] = {}
        self._ann_stack: List[Optional[ast.expr]] = []  # current top-level annotation node
        self._in_annotation = False
        self._is_string = False

    def _lookup(self, chain: List[str]) -> Optional[str]:
        for mk in self._sorted_keys:
            parts = mk.split(".")
            if chain[: len(parts)] == parts:
                return self._mk_to_full[mk]
        return None

    def _record(self, chain: List[str]) -> None:
        full = self._lookup(chain)
        if full is None:
            return
        if not self._in_annotation:
            self.runtime_use.add(full)
        else:
            top_ann = self._ann_stack[-1] if self._ann_stack else None
            self.annotation_nodes.setdefault(full, []).append((top_ann, self._is_string))

    # Annotation-context helpers -------------------------------------------

    def _visit_annotation(self, node: Optional[ast.expr]) -> None:
        if node is None:
            return
        old_ann, old_str = self._in_annotation, self._is_string
        self._in_annotation = True
        top_node = node
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            self._is_string = True
            self._ann_stack.append(node)
            try:
                inner = ast.parse(node.value, mode="eval")
                self.visit(inner)
            except SyntaxError:
                pass
            self._ann_stack.pop()
        else:
            self._ann_stack.append(top_node)
            self.visit(node)
            self._ann_stack.pop()
        self._in_annotation, self._is_string = old_ann, old_str

    def _visit_runtime(self, node: Optional[ast.expr]) -> None:
        if node is None:
            return
        old = self._in_annotation
        self._in_annotation = False
        self.visit(node)
        self._in_annotation = old

    # Skip TYPE_CHECKING bodies ----------------------------------------------

    def visit_If(self, node: ast.If) -> None:
        if _is_type_checking_guard(node.test):
            for s in node.orelse:
                self.visit(s)
        else:
            self.generic_visit(node)

    # Functions / classes ----------------------------------------------------

    def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
        for d in node.decorator_list:
            self._visit_runtime(d)
        all_args = (
            node.args.posonlyargs
            + node.args.args
            + node.args.kwonlyargs
            + ([node.args.vararg] if node.args.vararg else [])
            + ([node.args.kwarg] if node.args.kwarg else [])
        )
        for arg in all_args:
            self._visit_annotation(arg.annotation)
        for dflt in node.args.defaults + node.args.kw_defaults:
            if dflt is not None:
                self._visit_runtime(dflt)
        self._visit_annotation(node.returns)
        old = self._in_annotation
        self._in_annotation = False
        for s in node.body:
            self.visit(s)
        self._in_annotation = old

    visit_AsyncFunctionDef = visit_FunctionDef  # type: ignore[assignment]

    def visit_ClassDef(self, node: ast.ClassDef) -> None:
        for base in node.bases:
            self._visit_runtime(base)
        for kw in node.keywords:
            self._visit_runtime(kw.value)
        for d in node.decorator_list:
            self._visit_runtime(d)
        for s in node.body:
            self.visit(s)

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:
        self._visit_annotation(node.annotation)
        if node.value is not None:
            self._visit_runtime(node.value)

    def visit_Call(self, node: ast.Call) -> None:
        fn = ".".join(_dotted_chain(node.func) or [])
        self._visit_runtime(node.func)
        for i, arg in enumerate(node.args):
            if fn in ("isinstance", "issubclass") and i == 1:
                self._visit_runtime(arg)
            elif fn in ("cast", "typing.cast") and i == 0:
                self._visit_runtime(arg)
            else:
                self.visit(arg)
        for kw in node.keywords:
            self.visit(kw.value)

    # Name / Attribute -------------------------------------------------------

    def visit_Name(self, node: ast.Name) -> None:
        self._record([node.id])

    def visit_Attribute(self, node: ast.Attribute) -> None:
        chain = _dotted_chain(node)
        if chain and self._lookup(chain):
            self._record(chain)
            return  # don't visit children (would double-count root Name)
        self.generic_visit(node)


def _all_exports(tree: ast.Module) -> Set[str]:
    """Return all names listed in top-level __all__ = [...] assignments."""
    exports: Set[str] = set()
    for stmt in tree.body:
        if not isinstance(stmt, ast.Assign):
            continue
        for target in stmt.targets:
            if isinstance(target, ast.Name) and target.id == "__all__":
                if isinstance(stmt.value, (ast.List, ast.Tuple)):
                    for elt in stmt.value.elts:
                        if isinstance(elt, ast.Constant) and isinstance(elt.value, str):
                            exports.add(elt.value)
    return exports


def filter_true_candidates(
    candidates: Dict[str, CandidateImport], tree: ast.Module
) -> Dict[str, CandidateImport]:
    """Remove candidates that actually have runtime usages."""
    mk_map = {c.match_key: fm for fm, c in candidates.items()}
    clf = UsageClassifier(mk_map)
    clf.visit(tree)

    # Names re-exported via __all__ are runtime uses even if not referenced in code.
    exported = _all_exports(tree)
    for fm, c in candidates.items():
        # The name as it exists in this module's namespace is the alias (match_key for
        # from-imports) or the first component for bare `import spack.foo` statements.
        exposed_name = c.match_key.split(".")[0] if "." in c.match_key else c.match_key
        if exposed_name in exported:
            clf.runtime_use.add(fm)

    return {fm: c for fm, c in candidates.items() if fm not in clf.runtime_use}, clf


# ---------------------------------------------------------------------------
# Step 3 – build text edits
# ---------------------------------------------------------------------------


def _extract_text(lines: List[str], node: ast.expr) -> str:
    """Extract source text for a node (handles multi-line by collapsing)."""
    sl, sc = node.lineno - 1, node.col_offset
    el, ec = node.end_lineno - 1, node.end_col_offset  # type: ignore[attr-defined]
    if sl == el:
        return lines[sl][sc:ec]
    # Multi-line: join and collapse internal whitespace.
    parts = [lines[sl][sc:]]
    for i in range(sl + 1, el):
        parts.append(lines[i].strip())
    parts.append(lines[el][:ec].strip())
    return " ".join(p.strip() for p in parts if p.strip())


def _quote(text: str) -> str:
    """Wrap annotation text in double-quotes, escaping existing quotes."""
    escaped = text.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def _import_stmt_end_col(node: ast.stmt, source_lines: List[str]) -> int:
    """End col for a deletion edit that covers the full import line(s)."""
    end_line_idx = node.end_lineno - 1  # type: ignore[attr-defined]
    return len(source_lines[end_line_idx])


def build_edits(
    tree: ast.Module,
    source_lines: List[str],
    true_candidates: Dict[str, CandidateImport],
    classifier: UsageClassifier,
) -> Tuple[List[TextEdit], List[str]]:
    """
    Return (text_edits, guarded_import_texts).

    text_edits cover:
      - import statement modifications (full deletion or removal of candidate names)
      - annotation stringification

    guarded_import_texts are the lines to place inside ``if TYPE_CHECKING:``.
    """
    from collections import defaultdict

    edits: List[TextEdit] = []
    guarded_texts: List[str] = []

    # ---- Group candidates by their stmt_node so we process each statement
    # exactly once (multiple from-import aliases can share a node). --------
    stmt_to_cands: Dict[int, List[CandidateImport]] = defaultdict(list)
    for cand in true_candidates.values():
        stmt_to_cands[id(cand.stmt_node)].append(cand)

    for _stmt_id, cands in stmt_to_cands.items():
        node = cands[0].stmt_node

        if isinstance(node, ast.Import):
            # `import a.b` or `import a.b as x` — one alias per statement in
            # canonical Spack code.  Handle multi-alias just in case.
            candidate_aliases = {c.match_key for c in cands}
            all_aliases = [a for a in node.names]
            remaining = [a for a in all_aliases if (a.asname or a.name) not in candidate_aliases]

            if not remaining:
                # All aliases are candidates → blank the whole statement
                # (keep \n so line numbers stay intact for the re-parse).
                edits.append(
                    TextEdit(
                        start_line=node.lineno,
                        start_col=0,
                        end_line=node.end_lineno,  # type: ignore[attr-defined]
                        end_col=_import_stmt_end_col(node, source_lines),
                        replacement="\n",
                    )
                )
            else:
                # Partial: rewrite with remaining aliases only.
                new_stmt = "import " + ", ".join(
                    (f"{a.name} as {a.asname}" if a.asname else a.name) for a in remaining
                )
                edits.append(
                    TextEdit(
                        start_line=node.lineno,
                        start_col=0,
                        end_line=node.end_lineno,  # type: ignore[attr-defined]
                        end_col=_import_stmt_end_col(node, source_lines),
                        replacement=new_stmt + "\n",
                    )
                )

            for cand in cands:
                guarded_texts.append(cand.guarded_text)

        elif isinstance(node, ast.ImportFrom):
            module = node.module or ""
            candidate_names: Set[str] = {
                c.imported_name for c in cands if c.imported_name is not None
            }
            all_aliases = [a for a in node.names if a.name != "*"]
            remaining = [a for a in all_aliases if a.name not in candidate_names]

            if not remaining:
                # All symbols → blank the whole statement
                # (keep \n so line numbers stay intact for the re-parse).
                edits.append(
                    TextEdit(
                        start_line=node.lineno,
                        start_col=0,
                        end_line=node.end_lineno,  # type: ignore[attr-defined]
                        end_col=_import_stmt_end_col(node, source_lines),
                        replacement="\n",
                    )
                )
            else:
                # Partial: rewrite with remaining symbols on one line.
                parts = []
                for a in remaining:
                    parts.append(f"{a.name} as {a.asname}" if a.asname else a.name)
                new_stmt = f"from {module} import {', '.join(parts)}"
                edits.append(
                    TextEdit(
                        start_line=node.lineno,
                        start_col=0,
                        end_line=node.end_lineno,  # type: ignore[attr-defined]
                        end_col=_import_stmt_end_col(node, source_lines),
                        replacement=new_stmt + "\n",
                    )
                )

            for cand in cands:
                guarded_texts.append(cand.guarded_text)

    # ---- Annotation stringification: one edit per annotation node --------
    # Collect (full_module, ann_node, is_string) for all true candidates.
    seen_ann_spans: Set[Tuple[int, int, int, int]] = set()
    for full_module in true_candidates:
        for ann_node, is_string in classifier.annotation_nodes.get(full_module, []):
            if is_string or ann_node is None:
                continue
            span = (
                ann_node.lineno,
                ann_node.col_offset,
                ann_node.end_lineno,  # type: ignore[attr-defined]
                ann_node.end_col_offset,  # type: ignore[attr-defined]
            )
            if span in seen_ann_spans:
                continue  # same annotation references multiple candidates — stringify once
            seen_ann_spans.add(span)
            text = _extract_text(source_lines, ann_node)
            edits.append(
                TextEdit(
                    start_line=ann_node.lineno,
                    start_col=ann_node.col_offset,
                    end_line=ann_node.end_lineno,  # type: ignore[attr-defined]
                    end_col=ann_node.end_col_offset,  # type: ignore[attr-defined]
                    replacement=_quote(text),
                )
            )

    return edits, list(dict.fromkeys(guarded_texts))  # preserve order, deduplicate


# ---------------------------------------------------------------------------
# Step 4 – apply edits + insert into TYPE_CHECKING block
# ---------------------------------------------------------------------------


def apply_edits(source_lines: List[str], edits: List[TextEdit]) -> List[str]:
    """Apply TextEdits bottom-to-top, returning the modified lines."""
    # Sort: last line first, then last col first, to preserve offsets.
    for edit in sorted(edits, key=lambda e: (-e.start_line, -e.start_col)):
        sl, sc = edit.start_line - 1, edit.start_col
        el, ec = edit.end_line - 1, edit.end_col

        if sl == el:
            line = source_lines[sl]
            source_lines[sl] = line[:sc] + edit.replacement + line[ec:]
        else:
            # Multi-line edit: replace from (sl, sc) to (el, ec).
            # Preserve line count by keeping extra lines as bare "\n" so that
            # subsequent line-number references in insert_type_checking stay valid.
            first = source_lines[sl][:sc] + edit.replacement
            last = source_lines[el][ec:]
            combined = first + last
            source_lines[sl] = combined
            for i in range(sl + 1, el + 1):
                source_lines[i] = "\n"

    return source_lines


def _indent(node: ast.If) -> str:
    """Return the body indentation of a TYPE_CHECKING if-block."""
    if node.body:
        first = node.body[0]
        return " " * (first.col_offset)
    return "    "


def insert_type_checking(
    source_lines: List[str], tree: ast.Module, imports_to_add: List[str]
) -> List[str]:
    """
    Add *imports_to_add* to the ``if TYPE_CHECKING:`` block.
    If no such block exists, create one after the last top-level import.
    Also ensures ``from typing import TYPE_CHECKING`` is present.
    """
    if not imports_to_add:
        return source_lines

    lines = list(source_lines)

    # ---- Find existing TYPE_CHECKING block --------------------------------
    tc_node: Optional[ast.If] = None
    for stmt in tree.body:
        if isinstance(stmt, ast.If) and _is_type_checking_guard(stmt.test):
            tc_node = stmt
            break

    if tc_node is not None:
        indent = _indent(tc_node)
        # Insert after the last statement in the block body.
        last_body_stmt = tc_node.body[-1]
        insert_after_line = last_body_stmt.end_lineno  # type: ignore[attr-defined]
        new_lines = [f"{indent}{imp}\n" for imp in sorted(imports_to_add)]
        lines[insert_after_line:insert_after_line] = new_lines
    else:
        # ---- Create a new TYPE_CHECKING block -----------------------------
        # Find the last top-level import line.
        last_import_line = 0
        for stmt in tree.body:
            if isinstance(stmt, (ast.Import, ast.ImportFrom)):
                last_import_line = stmt.end_lineno  # type: ignore[attr-defined]
            elif isinstance(stmt, ast.If) and _is_type_checking_guard(stmt.test):
                pass  # already handled above

        if last_import_line == 0:
            last_import_line = 1

        block_lines = ["\n", "if TYPE_CHECKING:\n"]
        for imp in sorted(imports_to_add):
            block_lines.append(f"    {imp}\n")
        lines[last_import_line:last_import_line] = block_lines

    # ---- Ensure TYPE_CHECKING is imported ----------------------------------
    lines = ensure_type_checking_import(lines, tree)

    return lines


def ensure_type_checking_import(lines: List[str], tree: ast.Module) -> List[str]:
    """
    Make sure ``from typing import TYPE_CHECKING`` (or equivalent) exists.
    Inserts it after existing typing imports if missing.
    """
    # Check whether it's already there.
    for stmt in tree.body:
        if isinstance(stmt, ast.ImportFrom):
            if stmt.module == "typing":
                for alias in stmt.names:
                    if alias.name == "TYPE_CHECKING":
                        return lines  # already present
        if isinstance(stmt, ast.If) and _is_type_checking_guard(stmt.test):
            return lines  # guard exists → must have been imported

    # Find where to insert: after the last `from typing import` or `import typing`.
    insert_after = 0
    for stmt in tree.body:
        if isinstance(stmt, ast.ImportFrom) and stmt.module == "typing":
            insert_after = stmt.end_lineno  # type: ignore[attr-defined]
        elif isinstance(stmt, ast.Import):
            for alias in stmt.names:
                if alias.name == "typing":
                    insert_after = stmt.end_lineno  # type: ignore[attr-defined]

    if insert_after == 0:
        # Fallback: after the last stdlib import block.
        for stmt in tree.body:
            if isinstance(stmt, (ast.Import, ast.ImportFrom)):
                insert_after = stmt.end_lineno  # type: ignore[attr-defined]

    lines_copy = list(lines)
    lines_copy.insert(insert_after, "from typing import TYPE_CHECKING\n")
    return lines_copy


# ---------------------------------------------------------------------------
# Main per-file orchestration
# ---------------------------------------------------------------------------


def fix_file(path: Path, dry_run: bool = False) -> Optional[Dict]:
    source = path.read_text(encoding="utf-8", errors="replace")
    source_lines = source.splitlines(keepends=True)

    try:
        tree = ast.parse(source, filename=str(path))
    except SyntaxError as e:
        print(f"  SKIP (parse error): {e}", file=sys.stderr)
        return None

    # Collect spack.* candidates.
    candidates = collect_candidates(tree, source_lines)
    if not candidates:
        return None

    # Filter out any with actual runtime uses.
    true_candidates, classifier = filter_true_candidates(candidates, tree)
    if not true_candidates:
        return None

    # Build and apply text edits (import removal + annotation stringification).
    edits, guarded_texts = build_edits(tree, source_lines, true_candidates, classifier)
    if not edits and not guarded_texts:
        return None

    modified_lines = apply_edits(list(source_lines), edits)

    # Re-parse the modified source to get accurate line positions for the
    # TYPE_CHECKING insertion (import lines have shifted).
    modified_source = "".join(modified_lines)
    try:
        modified_tree = ast.parse(modified_source, filename=str(path))
    except SyntaxError:
        # If the modified source is broken, bail out for this file.
        print(f"  SKIP (post-edit parse error in {path.name})", file=sys.stderr)
        return None

    final_lines = insert_type_checking(modified_lines, modified_tree, guarded_texts)

    final_source = "".join(final_lines)

    if dry_run:
        diff = "".join(
            difflib.unified_diff(
                source_lines, final_lines, fromfile=f"a/{path.name}", tofile=f"b/{path.name}"
            )
        )
        if diff:
            print(diff)
    else:
        path.write_text(final_source, encoding="utf-8")

    return {
        "path": str(path),
        "moved": [c.guarded_text for c in true_candidates.values()],
        "bare_stringified": sum(
            1
            for fm in true_candidates
            for _, is_str in classifier.annotation_nodes.get(fm, [])
            if not is_str
        ),
    }


# ---------------------------------------------------------------------------
# Directory walk
# ---------------------------------------------------------------------------


def collect_files(root: Path) -> List[Path]:
    files: List[Path] = []
    for dirpath, dirnames, filenames in root.walk():
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        for fname in sorted(filenames):
            if fname.endswith(".py"):
                files.append(dirpath / fname)
    return files


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("path", nargs="?", default=str(DEFAULT_ROOT))
    parser.add_argument("--dry-run", action="store_true", help="print diffs, don't write")
    parser.add_argument("--json-report", metavar="FILE", help="write JSON summary here")
    args = parser.parse_args()

    root = Path(args.path)
    if not root.exists():
        sys.exit(f"Path not found: {root}")

    files = [root] if root.is_file() else collect_files(root)
    if not files:
        sys.exit("No Python files found.")

    results = []
    changed = 0
    for path in files:
        try:
            result = fix_file(path, dry_run=args.dry_run)
        except Exception as exc:
            print(f"  ERROR  {path}: {exc}", file=sys.stderr)
            continue
        if result:
            changed += 1
            results.append(result)
            if not args.dry_run:
                rel = path.relative_to(root.parent.parent.parent) if root.is_dir() else path
                moved = ", ".join(result["moved"])
                print(f"  fixed  {rel}  ({moved})")

    verb = "Would fix" if args.dry_run else "Fixed"
    print(f"\n{verb} {changed} file(s).")

    if args.json_report:
        with open(args.json_report, "w") as f:
            json.dump(results, f, indent=2)
        print(f"Report written to {args.json_report}")


if __name__ == "__main__":
    main()
