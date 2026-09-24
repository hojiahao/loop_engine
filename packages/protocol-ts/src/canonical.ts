import { createHash, timingSafeEqual } from "node:crypto";

import {
  assert_sha256_id,
  type CanonicalizationLimitOverrides,
  type CanonicalizationLimits,
  decode_factor_ast,
  decode_factor_spec,
  FACTOR_AST_SCHEMA,
  FACTOR_SPEC_POLICY_FIELDS,
  FACTOR_SPEC_SCHEMA,
  type FactorAst,
  FactorDomainError,
  type FactorSpec,
  type OperatorPolicyRegistry,
  type PolicyRef,
  resolve_canonicalization_limits,
  type Sha256Id,
  type ValueType,
} from "./domain.js";

declare const expressionIdBrand: unique symbol;
declare const factorSpecIdBrand: unique symbol;
const boundFactorSpecBrand: unique symbol = Symbol("bound-factor-spec");

export type ExpressionId = Sha256Id & { readonly [expressionIdBrand]: true };
export type FactorSpecId = Sha256Id & { readonly [factorSpecIdBrand]: true };

export interface CanonicalExpression {
  readonly ast: FactorAst;
  readonly valueType: ValueType;
  readonly canonicalJson: string;
  readonly expressionId: ExpressionId;
  to_bytes(): Uint8Array;
}

export interface CanonicalFactorSpec {
  readonly [boundFactorSpecBrand]: true;
  readonly spec: FactorSpec;
  readonly expression: CanonicalExpression;
  readonly canonicalJson: string;
  readonly factorSpecId: FactorSpecId;
  to_bytes(): Uint8Array;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

interface NormalizedNode {
  readonly ast: FactorAst;
  readonly type: ValueType;
}

export function prepare_expression(
  input: unknown,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const limits = resolve_canonicalization_limits(limitOverrides);
  const submitted = decode_factor_ast(input, limits);
  const normalized = normalize_node(submitted, registry, limits, "$").ast;
  validate_tree_limits(normalized, limits);
  const canonicalJson = write_canonical_expression(normalized);
  assert_byte_limit(canonicalJson, limits);

  // The evaluator-facing value always comes from reparsing the exact identity bytes.
  return parse_canonical_expression(encoder.encode(canonicalJson), registry, limits);
}

export function parse_canonical_expression(
  input: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const limits = resolve_canonicalization_limits(limitOverrides);
  const { bytes, text } = decode_utf8(input, limits, "$expression_bytes");
  let untyped: unknown;
  try {
    untyped = JSON.parse(text);
  } catch (error) {
    throw new FactorDomainError(
      "invalid_json",
      "$expression_bytes",
      error instanceof Error ? error.message : "invalid JSON",
    );
  }

  const parsed = decode_factor_ast(untyped, limits);
  const normalizedResult = normalize_node(parsed, registry, limits, "$");
  const normalized = normalizedResult.ast;
  validate_tree_limits(normalized, limits);
  const canonicalJson = write_canonical_expression(normalized);
  const canonicalBytes = encoder.encode(canonicalJson);
  if (!equal_bytes(bytes, canonicalBytes)) {
    throw new FactorDomainError(
      "non_canonical",
      "$expression_bytes",
      "bytes differ from the dedicated canonical writer output",
    );
  }

  const expressionId = hash_identity(FACTOR_AST_SCHEMA, canonicalBytes) as ExpressionId;
  return Object.freeze({
    ast: normalized,
    valueType: normalizedResult.type,
    canonicalJson,
    expressionId,
    to_bytes: () => encoder.encode(canonicalJson),
  });
}

export function bind_factor_spec(
  input: unknown,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  const spec = decode_factor_spec(input);
  const expression = parse_canonical_expression(canonicalExpression, registry, limitOverrides);
  assert_identity_matches(
    assert_sha256_id(spec.operator_registry_sha256, "$factor_spec.operator_registry_sha256"),
    registry.sha256,
    "$factor_spec.operator_registry_sha256",
  );
  assert_identity_matches(
    assert_sha256_id(spec.expression_id, "$factor_spec.expression_id"),
    expression.expressionId,
    "$factor_spec.expression_id",
  );
  if (expression.valueType !== "series") {
    throw new FactorDomainError(
      "type_mismatch",
      "$factor_spec.expression_id",
      `factor root must resolve to series, received ${value_type_label(expression.valueType)}`,
    );
  }
  const canonicalJson = write_factor_spec(spec);
  const canonicalBytes = encoder.encode(canonicalJson);
  const factorSpecId = hash_identity(FACTOR_SPEC_SCHEMA, canonicalBytes) as FactorSpecId;
  return Object.freeze({
    [boundFactorSpecBrand]: true as const,
    spec,
    expression,
    canonicalJson,
    factorSpecId,
    to_bytes: () => encoder.encode(canonicalJson),
  });
}

export function parse_factor_spec(
  input: Uint8Array | string,
  expectedFactorSpecId: string,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  const limits = resolve_canonicalization_limits(limitOverrides);
  const { bytes, text } = decode_utf8(input, limits, "$factor_spec_bytes");
  let untyped: unknown;
  try {
    untyped = JSON.parse(text);
  } catch (error) {
    throw new FactorDomainError(
      "invalid_json",
      "$factor_spec_bytes",
      error instanceof Error ? error.message : "invalid JSON",
    );
  }
  const spec = decode_factor_spec(untyped);
  const canonicalJson = write_factor_spec(spec);
  const canonicalBytes = encoder.encode(canonicalJson);
  if (!equal_bytes(bytes, canonicalBytes)) {
    throw new FactorDomainError(
      "non_canonical",
      "$factor_spec_bytes",
      "bytes differ from the dedicated canonical writer output",
    );
  }
  const bound = bind_factor_spec(spec, canonicalExpression, registry, limits);
  assert_identity_matches(
    assert_sha256_id(expectedFactorSpecId, "$expected_factor_spec_id"),
    bound.factorSpecId,
    "$expected_factor_spec_id",
  );
  return bound;
}

export function verify_expression_identity(
  expected: string,
  input: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const expectedId = assert_sha256_id(expected, "$expected_expression_id");
  const canonical = parse_canonical_expression(input, registry, limitOverrides);
  assert_identity_matches(expectedId, canonical.expressionId, "$expected_expression_id");
  return canonical;
}

export function verify_factor_identity(
  expected: string,
  input: Uint8Array | string,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  return parse_factor_spec(input, expected, canonicalExpression, registry, limitOverrides);
}

function value_type_label(valueType: ValueType): string {
  return typeof valueType === "string" ? valueType : `enum:${valueType.enumType}`;
}

function normalize_node(
  ast: FactorAst,
  registry: OperatorPolicyRegistry,
  limits: Readonly<CanonicalizationLimits>,
  path: string,
): NormalizedNode {
  switch (ast.node) {
    case "field":
      return Object.freeze({ ast, type: registry.resolve_field(ast.field, `${path}.field`) });
    case "decimal":
      return Object.freeze({ ast, type: "decimal" });
    case "boolean":
      return Object.freeze({ ast, type: "boolean" });
    case "enum":
      return Object.freeze({
        ast,
        type: registry.resolve_enum(ast.enum_type, ast.value, path),
      });
    case "call": {
      const definition = registry.resolve_operator(
        ast.operator,
        ast.operator_version,
        `${path}.operator`,
      );
      let arguments_ = ast.arguments.map((argument, index) =>
        normalize_node(argument, registry, limits, `${path}.arguments[${index}]`),
      );
      if (!definition.associative) {
        registry.validate_arguments(definition, arguments_, `${path}.arguments`);
      }

      if (definition.associative) {
        arguments_ = arguments_.flatMap((argument) => {
          const child = argument.ast;
          if (
            child.node === "call" &&
            child.operator === ast.operator &&
            child.operator_version === ast.operator_version
          ) {
            return child.arguments.map((grandchild) => ({
              ast: grandchild,
              type: definition.outputType,
            }));
          }
          return [argument];
        });
      }
      if (arguments_.length > limits.maxDirectArguments) {
        throw new FactorDomainError(
          "limit_exceeded",
          `${path}.arguments`,
          `normalization produced more than ${limits.maxDirectArguments} direct arguments`,
        );
      }
      if (definition.commutative) {
        arguments_.sort((left, right) =>
          compare_bytes(
            encoder.encode(write_canonical_expression(left.ast)),
            encoder.encode(write_canonical_expression(right.ast)),
          ),
        );
      }
      registry.validate_arguments(definition, arguments_, `${path}.arguments`);
      const normalizedAst: FactorAst = Object.freeze({
        node: "call",
        operator: ast.operator,
        operator_version: ast.operator_version,
        arguments: Object.freeze(arguments_.map((argument) => argument.ast)),
      });
      return Object.freeze({ ast: normalizedAst, type: definition.outputType });
    }
  }
}

function validate_tree_limits(ast: FactorAst, limits: Readonly<CanonicalizationLimits>): void {
  let nodes = 0;
  const visit = (node: FactorAst, depth: number): void => {
    nodes += 1;
    if (nodes > limits.maxNodes) {
      throw new FactorDomainError(
        "limit_exceeded",
        "$",
        `canonical AST exceeds ${limits.maxNodes} nodes`,
      );
    }
    if (depth > limits.maxDepth) {
      throw new FactorDomainError(
        "limit_exceeded",
        "$",
        `canonical AST exceeds ${limits.maxDepth} levels`,
      );
    }
    if (node.node === "call") {
      if (node.arguments.length > limits.maxDirectArguments) {
        throw new FactorDomainError(
          "limit_exceeded",
          "$.arguments",
          `call exceeds ${limits.maxDirectArguments} direct arguments`,
        );
      }
      for (const argument of node.arguments) {
        visit(argument, depth + 1);
      }
    }
  };
  visit(ast, 1);
}

function write_canonical_expression(ast: FactorAst): string {
  switch (ast.node) {
    case "field":
      return `{"node":"field","field":"${ast.field}"}`;
    case "decimal":
      return `{"node":"decimal","value":"${ast.value}"}`;
    case "boolean":
      return `{"node":"boolean","value":${ast.value ? "true" : "false"}}`;
    case "enum":
      return `{"node":"enum","enum_type":"${ast.enum_type}","value":"${ast.value}"}`;
    case "call": {
      const argumentsJson = ast.arguments.map((argument) => write_canonical_expression(argument));
      return `{"node":"call","operator":"${ast.operator}","operator_version":"${ast.operator_version}","arguments":[${argumentsJson.join(",")}]}`;
    }
  }
}

function write_policy_ref(policy: PolicyRef): string {
  return `{"policy_id":"${policy.policy_id}","revision":"${policy.revision}","sha256":"${policy.sha256}"}`;
}

function write_factor_spec(spec: FactorSpec): string {
  const policyFields = FACTOR_SPEC_POLICY_FIELDS.map(
    (field) => `"${field}":${write_policy_ref(spec[field])}`,
  );
  return `{"schema":"${FACTOR_SPEC_SCHEMA}","expression_id":"${spec.expression_id}","operator_registry_sha256":"${spec.operator_registry_sha256}","direction":"${spec.direction}",${policyFields.join(",")}}`;
}

function decode_utf8(
  input: Uint8Array | string,
  limits: Readonly<CanonicalizationLimits>,
  path: string,
): { readonly bytes: Uint8Array; readonly text: string } {
  const bytes = typeof input === "string" ? encoder.encode(input) : new Uint8Array(input);
  if (bytes.byteLength > limits.maxCanonicalBytes) {
    throw new FactorDomainError(
      "limit_exceeded",
      path,
      `canonical bytes exceed ${limits.maxCanonicalBytes}`,
    );
  }
  let text: string;
  try {
    text = decoder.decode(bytes);
  } catch (error) {
    throw new FactorDomainError(
      "invalid_json",
      path,
      error instanceof Error ? error.message : "invalid UTF-8",
    );
  }
  return { bytes, text };
}

function assert_byte_limit(text: string, limits: Readonly<CanonicalizationLimits>): void {
  const actual = encoder.encode(text).byteLength;
  if (actual > limits.maxCanonicalBytes) {
    throw new FactorDomainError(
      "limit_exceeded",
      "$",
      `canonical AST has ${actual} bytes; maximum is ${limits.maxCanonicalBytes}`,
    );
  }
}

function hash_identity(domain: string, canonicalBytes: Uint8Array): Sha256Id {
  const hash = createHash("sha256");
  hash.update(domain, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(canonicalBytes);
  return `sha256:${hash.digest("hex")}` as Sha256Id;
}

function assert_identity_matches(expected: Sha256Id, actual: Sha256Id, path: string): void {
  const expectedBytes = Buffer.from(expected.slice("sha256:".length), "hex");
  const actualBytes = Buffer.from(actual.slice("sha256:".length), "hex");
  if (!timingSafeEqual(expectedBytes, actualBytes)) {
    throw new FactorDomainError(
      "identity_mismatch",
      path,
      `expected ${expected}, computed ${actual}`,
    );
  }
}

function equal_bytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && compare_bytes(left, right) === 0;
}

function compare_bytes(left: Uint8Array, right: Uint8Array): number {
  const sharedLength = Math.min(left.byteLength, right.byteLength);
  for (let index = 0; index < sharedLength; index += 1) {
    const leftByte = left[index];
    const rightByte = right[index];
    if (leftByte === undefined || rightByte === undefined) {
      break;
    }
    if (leftByte !== rightByte) {
      return leftByte - rightByte;
    }
  }
  return left.byteLength - right.byteLength;
}
