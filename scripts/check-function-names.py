"""Check project-owned function declarations without adding a parser dependency."""

from __future__ import annotations

import ast
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAME = re.compile(r"[a-z][a-z0-9]*(?:_[a-z0-9]+){0,2}\Z", re.ASCII)
RUST = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?"
    r'(?:unsafe\s+)?(?:extern\s+"[^"]+"\s+)?fn\s+(?:r#)?(\w+)',
    re.M,
)
SHELL = re.compile(r"^\s*(?:function\s+)?([A-Za-z_]\w*)\s*\(\s*\)\s*\{", re.M)


def valid_name(name: str) -> bool:
    return NAME.fullmatch(name.strip("_")) is not None


def declarations(path: str, text: str) -> list[tuple[int, str]]:
    if path.endswith(".py"):
        return [
            (node.lineno, node.name)
            for node in ast.walk(ast.parse(text, filename=path))
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        ]
    pattern = RUST if path.endswith(".rs") else SHELL
    return [
        (text.count("\n", 0, match.start(1)) + 1, match[1])
        for match in pattern.finditer(text)
    ]


def main() -> int:
    paths = (
        subprocess.check_output(
            ["git", "ls-files", "-co", "--exclude-standard", "-z"], cwd=ROOT
        )
        .decode()
        .split("\0")
    )
    count = 0
    errors = []
    for path in sorted(set(paths)):
        if not path.endswith((".py", ".rs", ".sh")):
            continue
        if "/generated/" in path or path.startswith("python/loop_protocol/src/loop/"):
            continue
        if not (ROOT / path).is_file():
            continue
        for line, name in declarations(path, (ROOT / path).read_text()):
            count += 1
            if not valid_name(name):
                errors.append(
                    f"{path}:{line}: {name}: expected snake_case, at most three words"
                )
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Function names: {count} Python/Rust/Shell declarations passed.")
    return 0


class NamingTests(unittest.TestCase):
    def test_word_limit(self) -> None:
        for name in (
            "get",
            "_load_panel",
            "test_taf_cap",
            "__init_subclass__",
            "sha256_bytes",
        ):
            self.assertTrue(valid_name(name), name)
        for name in ("", "readPanel", "test_taf_per_order", "a__b", "a_b_c_d"):
            self.assertFalse(valid_name(name), name)

    def test_python_methods(self) -> None:
        source = "class Worker:\n    async def run_once(self):\n        pass\n# def ignored():\n"
        self.assertEqual(declarations("worker.py", source), [(2, "run_once")])

    def test_rust_forms(self) -> None:
        source = "pub(crate) async fn run_once() {}\n    fn trait_method();\n// fn ignored() {}\n"
        self.assertEqual(
            declarations("worker.rs", source), [(1, "run_once"), (2, "trait_method")]
        )

    def test_shell_forms(self) -> None:
        self.assertEqual(
            declarations("run.sh", "function run_once() {\n}\n"), [(1, "run_once")]
        )


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    elif sys.argv[1:]:
        raise SystemExit("usage: check-function-names.py [--self-test]")
    else:
        raise SystemExit(main())
