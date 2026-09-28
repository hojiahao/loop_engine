import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { collect_names, valid_name } from "./function-names.mjs";

test("word limit includes test prefix and permits private delimiters", () => {
  for (const name of ["get", "_load_panel", "test_taf_cap", "sha256_bytes"])
    assert.equal(valid_name(name), true, name);
  for (const name of ["", "readPanel", "test_taf_per_order", "a__b", "a_b_c_d"])
    assert.equal(valid_name(name), false, name);
});

test("AST finds callables and signatures without treating wire fields as methods", (t) => {
  const directory = mkdtempSync(join(tmpdir(), "loop-function-names-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "fixture.ts");
  writeFileSync(
    path,
    `// function fake_long_function_name() {}
export function valid_name() {}
const typed_arrow = async (value: string): Promise<string> => value;
const expression = function named_expression() {};
interface Reader {
  wireField: string;
  read_too_many_words(): void;
  invoke: (value: string) => void;
}
class Worker {
  get cache_bytes() { return ""; }
  set cache_bytes(value: string) {}
  run_once() {}
  arrow_method = () => true;
}
const object = { wireField: "kept", read_once() {}, callback: () => 1 };
`,
  );
  const names = collect_names([path]).map(({ name }) => name);
  assert.deepEqual(names, [
    "valid_name",
    "typed_arrow",
    "expression",
    "named_expression",
    "read_too_many_words",
    "invoke",
    "cache_bytes",
    "cache_bytes",
    "run_once",
    "arrow_method",
    "read_once",
    "callback",
  ]);
  assert.deepEqual(
    names.filter((name) => !valid_name(name)),
    ["read_too_many_words"],
  );
});
