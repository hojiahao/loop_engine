import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import * as ts from "typescript/unstable/ast";
import { API } from "typescript/unstable/sync";

const root = fileURLToPath(new URL("../../", import.meta.url));

function source_files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? source_files(path) : [path];
  });
}

function module_imports(source) {
  const result = new Set();
  function visit(node) {
    let value;
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) value = node.moduleSpecifier;
    else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument))
      value = node.argument.literal;
    else if (
      ts.isCallExpression(node) &&
      (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) && node.expression.text === "require") ||
        (ts.isCallExpression(node.expression) &&
          ts.isIdentifier(node.expression.expression) &&
          node.expression.expression.text === "createRequire"))
    )
      value = node.arguments[0];
    if (value && ts.isStringLiteral(value)) result.add(value.text);
    node.forEachChild(visit);
  }
  visit(source);
  return result;
}

test("Provider imports only its scoped wire and declared transport dependencies", (t) => {
  const package_data = JSON.parse(readFileSync(join(root, "apps/providerd/package.json"), "utf8"));
  const paths = source_files(join(root, "apps/providerd/src")).filter((path) =>
    path.endsWith(".ts"),
  );
  const api = new API({ cwd: root });
  t.after(() => api.close());
  const snapshot = api.updateSnapshot({ openFiles: paths });
  t.after(() => snapshot.dispose());
  for (const path of paths) {
    const source = snapshot.getDefaultProjectForFile(path)?.program.getSourceFile(path);
    assert.ok(source, path);
    for (const name of module_imports(source)) {
      if (name.startsWith("node:") || name.startsWith("./")) continue;
      if (name.startsWith("@loop-engine/"))
        assert.equal(name, "@loop-engine/protocol/provider", path);
      else
        assert.ok(
          Object.keys(package_data.dependencies).some(
            (dependency) => name === dependency || name.startsWith(`${dependency}/`),
          ),
          `${path}: ${name}`,
        );
    }
  }
});

test("Loop and numerical workers do not import supplier SDKs", (t) => {
  const forbidden =
    /(?:^|\n)\s*(?:use|import|from|extern crate)\s+(?:openai|anthropic|cohere|google\.genai|azure\.ai|aws_sdk_bedrockruntime)\b/;
  for (const directory of ["crates/loopd/src", "python/loop_research/src"]) {
    for (const path of source_files(join(root, directory)).filter((path) =>
      /\.(rs|py)$/.test(path),
    ))
      assert.doesNotMatch(readFileSync(path, "utf8"), forbidden, path);
  }
  assert.match("from openai import OpenAI", forbidden);
  assert.match("use aws_sdk_bedrockruntime::Client;", forbidden);
  const directory = mkdtempSync(join(tmpdir(), "loop-provider-boundary-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "fixture.ts");
  writeFileSync(
    path,
    'type T = typeof import("@sdk/types"); createRequire(import.meta.url)("@sdk/client");',
  );
  const api = new API({ cwd: root });
  t.after(() => api.close());
  const snapshot = api.updateSnapshot({ openFiles: [path] });
  t.after(() => snapshot.dispose());
  const source = snapshot.getDefaultProjectForFile(path)?.program.getSourceFile(path);
  assert.ok(source);
  assert.deepEqual([...module_imports(source)], ["@sdk/types", "@sdk/client"]);
});
