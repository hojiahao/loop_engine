import { createHash, timingSafeEqual } from "node:crypto";

import {
  assertSha256Id,
  type CanonicalizationLimitOverrides,
  type CanonicalizationLimits,
  decodeFactorAst,
  decodeFactorSpec,
  FACTOR_AST_SCHEMA,
  FACTOR_SPEC_POLICY_FIELDS,
  FACTOR_SPEC_SCHEMA,
  type FactorAst,
  FactorDomainError,
  type FactorSpec,
  type OperatorPolicyRegistry,
  type PolicyRef,
  resolveCanonicalizationLimits,
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
  toBytes(): Uint8Array;
}

export interface CanonicalFactorSpec {
  readonly [boundFactorSpecBrand]: true;
  readonly spec: FactorSpec;
  readonly expression: CanonicalExpression;
  readonly canonicalJson: string;
  readonly factorSpecId: FactorSpecId;
  toBytes(): Uint8Array;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

interface NormalizedNode {
  readonly ast: FactorAst;
  readonly type: ValueType;
}

export function prepareExpression(
  input: unknown,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const limits = resolveCanonicalizationLimits(limitOverrides);
  const submitted = decodeFactorAst(input, limits);
  const normalized = normalizeNode(submitted, registry, limits, "$").ast;
  validateTreeLimits(normalized, limits);
  const canonicalJson = writeCanonicalExpression(normalized);
  assertByteLimit(canonicalJson, limits);

  // The evaluator-facing value always comes from reparsing the exact identity bytes.
  return parseCanonicalExpression(encoder.encode(canonicalJson), registry, limits);
}

export function parseCanonicalExpression(
  input: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const limits = resolveCanonicalizationLimits(limitOverrides);
  const { bytes, text } = decodeUtf8(input, limits, "$expression_bytes");
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

  const parsed = decodeFactorAst(untyped, limits);
  const normalizedResult = normalizeNode(parsed, registry, limits, "$");
  const normalized = normalizedResult.ast;
  validateTreeLimits(normalized, limits);
  const canonicalJson = writeCanonicalExpression(normalized);
  const canonicalBytes = encoder.encode(canonicalJson);
  if (!equalBytes(bytes, canonicalBytes)) {
    throw new FactorDomainError(
      "non_canonical",
      "$expression_bytes",
      "bytes differ from the dedicated canonical writer output",
    );
  }

  const expressionId = hashIdentity(FACTOR_AST_SCHEMA, canonicalBytes) as ExpressionId;
  return Object.freeze({
    ast: normalized,
    valueType: normalizedResult.type,
    canonicalJson,
    expressionId,
    toBytes: () => encoder.encode(canonicalJson),
  });
}

export function bindFactorSpec(
  input: unknown,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  const spec = decodeFactorSpec(input);
  const expression = parseCanonicalExpression(canonicalExpression, registry, limitOverrides);
  assertIdentityMatches(
    assertSha256Id(spec.operator_registry_sha256, "$factor_spec.operator_registry_sha256"),
    registry.sha256,
    "$factor_spec.operator_registry_sha256",
  );
  assertIdentityMatches(
    assertSha256Id(spec.expression_id, "$factor_spec.expression_id"),
    expression.expressionId,
    "$factor_spec.expression_id",
  );
  if (expression.valueType !== "series") {
    throw new FactorDomainError(
      "type_mismatch",
      "$factor_spec.expression_id",
      `factor root must resolve to series, received ${valueTypeLabel(expression.valueType)}`,
    );
  }
  const canonicalJson = writeCanonicalFactorSpec(spec);
  const canonicalBytes = encoder.encode(canonicalJson);
  const factorSpecId = hashIdentity(FACTOR_SPEC_SCHEMA, canonicalBytes) as FactorSpecId;
  return Object.freeze({
    [boundFactorSpecBrand]: true as const,
    spec,
    expression,
    canonicalJson,
    factorSpecId,
    toBytes: () => encoder.encode(canonicalJson),
  });
}

export function parseCanonicalFactorSpec(
  input: Uint8Array | string,
  expectedFactorSpecId: string,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  const limits = resolveCanonicalizationLimits(limitOverrides);
  const { bytes, text } = decodeUtf8(input, limits, "$factor_spec_bytes");
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
  const spec = decodeFactorSpec(untyped);
  const canonicalJson = writeCanonicalFactorSpec(spec);
  const canonicalBytes = encoder.encode(canonicalJson);
  if (!equalBytes(bytes, canonicalBytes)) {
    throw new FactorDomainError(
      "non_canonical",
      "$factor_spec_bytes",
      "bytes differ from the dedicated canonical writer output",
    );
  }
  const bound = bindFactorSpec(spec, canonicalExpression, registry, limits);
  assertIdentityMatches(
    assertSha256Id(expectedFactorSpecId, "$expected_factor_spec_id"),
    bound.factorSpecId,
    "$expected_factor_spec_id",
  );
  return bound;
}

export function verifyExpressionIdentity(
  expected: string,
  input: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalExpression {
  const expectedId = assertSha256Id(expected, "$expected_expression_id");
  const canonical = parseCanonicalExpression(input, registry, limitOverrides);
  assertIdentityMatches(expectedId, canonical.expressionId, "$expected_expression_id");
  return canonical;
}

export function verifyFactorSpecIdentity(
  expected: string,
  input: Uint8Array | string,
  canonicalExpression: Uint8Array | string,
  registry: OperatorPolicyRegistry,
  limitOverrides: CanonicalizationLimitOverrides = {},
): CanonicalFactorSpec {
  return parseCanonicalFactorSpec(input, expected, canonicalExpression, registry, limitOverrides);
}

function valueTypeLabel(valueType: ValueType): string {
  return typeof valueType === "string" ? valueType : `enum:${valueType.enumType}`;
}

function normalizeNode(
  ast: FactorAst,
  registry: OperatorPolicyRegistry,
  limits: Readonly<CanonicalizationLimits>,
  path: string,
): NormalizedNode {
  switch (ast.node) {
    case "field":
      return Object.freeze({ ast, type: registry.resolveField(ast.field, `${path}.field`) });
    case "decimal":
      return Object.freeze({ ast, type: "decimal" });
    case "boolean":
      return Object.freeze({ ast, type: "boolean" });
    case "enum":
      return Object.freeze({
        ast,
        type: registry.resolveEnum(ast.enum_type, ast.value, path),
      });
    case "call": {
      const definition = registry.resolveOperator(
        ast.operator,
        ast.operator_version,
        `${path}.operator`,
      );
      let arguments_ = ast.arguments.map((argument, index) =>
        normalizeNode(argument, registry, limits, `${path}.arguments[${index}]`),
      );
      if (!definition.associative) {
        registry.validateArguments(definition, arguments_, `${path}.arguments`);
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
          compareBytes(
            encoder.encode(writeCanonicalExpression(left.ast)),
            encoder.encode(writeCanonicalExpression(right.ast)),
          ),
        );
      }
      registry.validateArguments(definition, arguments_, `${path}.arguments`);
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

function validateTreeLimits(ast: FactorAst, limits: Readonly<CanonicalizationLimits>): void {
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

function writeCanonicalExpression(ast: FactorAst): string {
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
      const argumentsJson = ast.arguments.map((argument) => writeCanonicalExpression(argument));
      return `{"node":"call","operator":"${ast.operator}","operator_version":"${ast.operator_version}","arguments":[${argumentsJson.join(",")}]}`;
    }
  }
}

function writePolicyRef(policy: PolicyRef): string {
  return `{"policy_id":"${policy.policy_id}","revision":"${policy.revision}","sha256":"${policy.sha256}"}`;
}

function writeCanonicalFactorSpec(spec: FactorSpec): string {
  const policyFields = FACTOR_SPEC_POLICY_FIELDS.map(
    (field) => `"${field}":${writePolicyRef(spec[field])}`,
  );
  return `{"schema":"${FACTOR_SPEC_SCHEMA}","expression_id":"${spec.expression_id}","operator_registry_sha256":"${spec.operator_registry_sha256}","direction":"${spec.direction}",${policyFields.join(",")}}`;
}

function decodeUtf8(
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

function assertByteLimit(text: string, limits: Readonly<CanonicalizationLimits>): void {
  const actual = encoder.encode(text).byteLength;
  if (actual > limits.maxCanonicalBytes) {
    throw new FactorDomainError(
      "limit_exceeded",
      "$",
      `canonical AST has ${actual} bytes; maximum is ${limits.maxCanonicalBytes}`,
    );
  }
}

function hashIdentity(domain: string, canonicalBytes: Uint8Array): Sha256Id {
  const hash = createHash("sha256");
  hash.update(domain, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(canonicalBytes);
  return `sha256:${hash.digest("hex")}` as Sha256Id;
}

function assertIdentityMatches(expected: Sha256Id, actual: Sha256Id, path: string): void {
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

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength && compareBytes(left, right) === 0;
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
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
