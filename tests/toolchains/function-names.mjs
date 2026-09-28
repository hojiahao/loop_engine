import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  isArrowFunction,
  isFunctionDeclaration,
  isFunctionExpression,
  isFunctionTypeNode,
  isGetAccessorDeclaration,
  isIdentifier,
  isMethodDeclaration,
  isMethodSignatureDeclaration,
  isPropertyAssignment,
  isPropertyDeclaration,
  isPropertySignatureDeclaration,
  isSetAccessorDeclaration,
  isVariableDeclaration,
} from "typescript/unstable/ast";
import { API } from "typescript/unstable/sync";

const root = fileURLToPath(new URL("../../", import.meta.url));

export function valid_name(name) {
  return /^[a-z][a-z0-9]*(?:_[a-z0-9]+){0,2}$/.test(name.replace(/^_+|_+$/g, ""));
}

// TypeScript 7's AST API is pinned with the workspace compiler. No transitive
// parser dependency or textual replacement of protocol fields is involved.
export function collect_names(paths) {
  const api = new API({ cwd: root });
  try {
    const snapshot = api.updateSnapshot({ openFiles: paths });
    const names = [];
    for (const path of paths) {
      const project = snapshot.getDefaultProjectForFile(path);
      const source = project?.program.getSourceFile(path);
      if (!source) throw new Error(`Cannot parse owned source: ${path}`);
      function visit(node) {
        const initializer = node.initializer;
        const callable =
          isFunctionDeclaration(node) ||
          isFunctionExpression(node) ||
          isMethodDeclaration(node) ||
          isMethodSignatureDeclaration(node) ||
          isGetAccessorDeclaration(node) ||
          isSetAccessorDeclaration(node) ||
          ((isVariableDeclaration(node) ||
            isPropertyAssignment(node) ||
            isPropertyDeclaration(node)) &&
            initializer &&
            (isArrowFunction(initializer) || isFunctionExpression(initializer))) ||
          (isPropertySignatureDeclaration(node) && node.type && isFunctionTypeNode(node.type));
        if (callable && node.name && isIdentifier(node.name)) {
          const line = source.getLineAndCharacterOfPosition(node.name.getStart(source)).line + 1;
          names.push({ path, line, name: node.name.text });
        }
        node.forEachChild(visit);
      }
      visit(source);
    }
    snapshot.dispose();
    return names;
  } finally {
    api.close();
  }
}

if (process.argv[2] === "--check") {
  const files = execFileSync("git", ["ls-files", "-co", "--exclude-standard", "-z"], {
    cwd: root,
    encoding: "utf8",
  });
  const paths = [...new Set(files.split("\0"))]
    .filter((path) => /\.(?:[cm]?[jt]s|tsx)$/.test(path) && !path.includes("/generated/"))
    .map((path) => resolve(root, path));
  const declarations = collect_names(paths);
  const invalid = declarations.filter(
    ({ path, name }) =>
      !valid_name(name) && !(path === resolve(root, "apps/web/src/App.tsx") && name === "App"),
  );
  for (const { path, line, name } of invalid) {
    console.error(`${path}:${line}: ${name}: expected snake_case, at most three words`);
  }
  if (invalid.length) process.exitCode = 1;
  else
    console.log(
      `Function names: ${declarations.length} TypeScript/JavaScript declarations passed.`,
    );
}
