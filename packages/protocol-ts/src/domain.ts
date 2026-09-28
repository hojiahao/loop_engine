import { createHash } from "node:crypto";

export const FACTOR_AST_SCHEMA = "loop.factor-ast/v1" as const;
export const FACTOR_SPEC_SCHEMA = "loop.factor-spec/v1" as const;
export const OPERATOR_REGISTRY_SCHEMA = "loop.operator-registry/v1" as const;
export const OPERATOR_SEMANTIC_CONTRACT_SCHEMA = "loop.operator-semantic-contract/v1" as const;

export const V1_HARD_LIMITS = Object.freeze({
  maxNodes: 4_096,
  maxDepth: 64,
  maxCanonicalBytes: 256 * 1_024,
  maxDirectArguments: 1_024,
});

export interface CanonicalizationLimits {
  readonly maxNodes: number;
  readonly maxDepth: number;
  readonly maxCanonicalBytes: number;
  readonly maxDirectArguments: number;
}

export type CanonicalizationLimitOverrides = Partial<CanonicalizationLimits>;

export type FactorAst = FieldNode | DecimalNode | BooleanNode | EnumNode | CallNode;

export interface FieldNode {
  readonly node: "field";
  readonly field: string;
}

export interface DecimalNode {
  readonly node: "decimal";
  readonly value: string;
}

export interface BooleanNode {
  readonly node: "boolean";
  readonly value: boolean;
}

export interface EnumNode {
  readonly node: "enum";
  readonly enum_type: string;
  readonly value: string;
}

export interface CallNode {
  readonly node: "call";
  readonly operator: string;
  readonly operator_version: string;
  readonly arguments: readonly FactorAst[];
}

export type FactorDirection = "higher_is_better" | "lower_is_better";

export type Sha256Id = `sha256:${string}`;

export interface PolicyRef {
  readonly policy_id: string;
  readonly revision: string;
  readonly sha256: Sha256Id;
}

export interface FactorSpec {
  readonly schema: typeof FACTOR_SPEC_SCHEMA;
  readonly expression_id: Sha256Id;
  readonly operator_registry_sha256: Sha256Id;
  readonly direction: FactorDirection;
  readonly universe_policy: PolicyRef;
  readonly data_policy: PolicyRef;
  readonly calendar_policy: PolicyRef;
  readonly preprocess_policy: PolicyRef;
  readonly neutralization_policy: PolicyRef;
  readonly portfolio_policy: PolicyRef;
  readonly execution_policy: PolicyRef;
  readonly cost_policy: PolicyRef;
  readonly evaluation_policy: PolicyRef;
}

export const FACTOR_SPEC_POLICY_FIELDS = [
  "universe_policy",
  "data_policy",
  "calendar_policy",
  "preprocess_policy",
  "neutralization_policy",
  "portfolio_policy",
  "execution_policy",
  "cost_policy",
  "evaluation_policy",
] as const;

export type FactorSpecPolicyField = (typeof FACTOR_SPEC_POLICY_FIELDS)[number];

export type ValueType = "series" | "decimal" | "boolean" | EnumValueType;

export interface EnumValueType {
  readonly enumType: string;
}

export interface DecimalConstraints {
  readonly maxPrecision: number;
  readonly maxScale: number;
  readonly minimum: string;
  readonly maximum: string;
}

export interface ArgumentDefinition {
  readonly type: ValueType;
  readonly literalOnly?: boolean;
  readonly decimal?: DecimalConstraints;
}

export interface FieldDefinition {
  readonly field: string;
  readonly outputType: ValueType;
}

export interface EnumDefinition {
  readonly enumType: string;
  readonly values: readonly string[];
}

export interface OperatorDefinition {
  readonly operator: string;
  readonly operatorVersion: string;
  readonly semanticContractSha256: Sha256Id;
  readonly parameters: readonly ArgumentDefinition[];
  readonly variadic?: ArgumentDefinition;
  readonly minArguments?: number;
  readonly maxArguments?: number;
  readonly outputType: ValueType;
  readonly associative: boolean;
  readonly commutative: boolean;
}

export interface OperatorRegistrySnapshot {
  readonly fields: readonly FieldDefinition[];
  readonly enums: readonly EnumDefinition[];
  readonly operators: readonly OperatorDefinition[];
}

export interface ResolvedOperatorDefinition {
  readonly operator: string;
  readonly operatorVersion: string;
  readonly semanticContractSha256: Sha256Id;
  readonly semanticContract: OperatorSemanticContract;
  readonly parameters: readonly ArgumentDefinition[];
  readonly variadic?: ArgumentDefinition;
  readonly minArguments: number;
  readonly maxArguments: number;
  readonly outputType: ValueType;
  readonly associative: boolean;
  readonly commutative: boolean;
}

export const NULL_POLICIES = [
  "not_applicable",
  "propagate",
  "ignore_missing",
  "preserve_target_ignore_peers",
  "reject_missing",
] as const;
export type NullPolicy = (typeof NULL_POLICIES)[number];

export const WINDOW_POLICIES = [
  "not_applicable",
  "trailing_argument_2_full_window_right_inclusive_constant_preserve",
  "trailing_argument_2_minimum_argument_3_right_inclusive",
  "trailing_argument_2_minimum_valid_min_n_max_3_floor_2n_div_3_right_inclusive_constant_preserve",
  "lag_argument_2",
] as const;
export type WindowPolicy = (typeof WINDOW_POLICIES)[number];

export const TIE_POLICIES = [
  "not_applicable",
  "average_valid_count",
  "dense_valid_count",
  "stable_first_valid_count_minus_one",
  "argument_2_average_or_dense_valid_count_constant_midpoint",
  "target_last_stable_order_valid_count_minus_one_constant_midpoint",
] as const;
export type TiePolicy = (typeof TIE_POLICIES)[number];

export const ALIGNMENT_POLICIES = [
  "not_applicable",
  "unary_preserve_timestamp_and_security",
  "strict_timestamp_and_security",
  "intersection_timestamp_and_security",
] as const;
export type AlignmentPolicy = (typeof ALIGNMENT_POLICIES)[number];

export const NUMERIC_POLICIES = [
  "not_applicable",
  "exact_decimal",
  "binary64_non_finite_to_missing",
  "binary64_reject_non_finite",
  "ordinal_unit_interval",
  "binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_zero_non_finite_to_missing",
  "binary64_sample_std_effective_n_minimum_2_non_finite_to_missing",
  "binary64_sample_zscore_effective_n_minimum_2_constant_missing",
  "binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_missing",
] as const;
export type NumericPolicy = (typeof NUMERIC_POLICIES)[number];

export interface OperatorSemanticContract {
  readonly schema: typeof OPERATOR_SEMANTIC_CONTRACT_SCHEMA;
  readonly operator: string;
  readonly operatorVersion: string;
  readonly nullPolicy: NullPolicy;
  readonly windowPolicy: WindowPolicy;
  readonly tiePolicy: TiePolicy;
  readonly alignmentPolicy: AlignmentPolicy;
  readonly numericPolicy: NumericPolicy;
}

export type SemanticContractResolver = (identity: Sha256Id) => Uint8Array | undefined;

export interface ResolvedArgument {
  readonly ast: FactorAst;
  readonly type: ValueType;
}

export type FactorDomainErrorCode =
  | "invalid_json"
  | "invalid_shape"
  | "invalid_identifier"
  | "invalid_decimal"
  | "invalid_digest"
  | "invalid_revision"
  | "unknown_field"
  | "unknown_enum"
  | "unknown_operator"
  | "type_mismatch"
  | "arity_mismatch"
  | "limit_exceeded"
  | "non_canonical"
  | "identity_mismatch"
  | "invalid_registry";

export class FactorDomainError extends Error {
  public override readonly name = "FactorDomainError";

  public constructor(
    public readonly code: FactorDomainErrorCode,
    public readonly path: string,
    message: string,
  ) {
    super(`${path}: ${message}`);
  }
}

const IDENTIFIER_PATTERN = /^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$/;
const POLICY_ID_PATTERN = /^[a-z][a-z0-9_.-]{0,127}$/;
const POSITIVE_UNSIGNED_DECIMAL_PATTERN = /^[1-9][0-9]*$/;
const CANONICAL_DECIMAL_PATTERN = /^-?(0|[1-9][0-9]*)(\.[0-9]*[1-9])?$/;
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const textEncoder = new TextEncoder();

function fail(code: FactorDomainErrorCode, path: string, message: string): never {
  throw new FactorDomainError(code, path, message);
}

function is_plain_object(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function require_object(value: unknown, path: string): Record<string, unknown> {
  if (!is_plain_object(value)) {
    fail("invalid_shape", path, "expected a plain object");
  }
  if (Object.getOwnPropertySymbols(value).length !== 0) {
    fail("invalid_shape", path, "symbol keys are not permitted");
  }
  return value;
}

function require_exact_keys(
  value: Record<string, unknown>,
  expected: readonly string[],
  path: string,
): void {
  const actual = Object.keys(value);
  const expectedSet = new Set(expected);
  for (const key of actual) {
    if (!expectedSet.has(key)) {
      fail("invalid_shape", `${path}.${key}`, "unknown field");
    }
  }
  for (const key of expected) {
    if (!Object.hasOwn(value, key)) {
      fail("invalid_shape", `${path}.${key}`, "missing required field");
    }
  }
}

function require_string(value: unknown, path: string): string {
  if (typeof value !== "string") {
    fail("invalid_shape", path, "expected a string");
  }
  return value;
}

function require_boolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    fail("invalid_shape", path, "expected a boolean");
  }
  return value;
}

function require_boolean_definition(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    fail("invalid_registry", path, "must be an explicit boolean");
  }
  return value;
}

function require_positive_integer(value: unknown, path: string, maximum: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1 || value > maximum) {
    fail("invalid_registry", path, `must be an integer from 1 through ${maximum}`);
  }
  return value;
}

function require_nonnegative_integer(value: unknown, path: string, maximum: number): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > maximum) {
    fail("invalid_registry", path, `must be an integer from 0 through ${maximum}`);
  }
  return value;
}

export function resolve_canonicalization_limits(
  overrides: CanonicalizationLimitOverrides = {},
): Readonly<CanonicalizationLimits> {
  const object = require_object(overrides, "$limits");
  const permitted = new Set(Object.keys(V1_HARD_LIMITS));
  for (const key of Object.keys(object)) {
    if (!permitted.has(key)) {
      fail("limit_exceeded", `$limits.${key}`, "unknown limit");
    }
  }

  const resolved = {
    maxNodes: overrides.maxNodes ?? V1_HARD_LIMITS.maxNodes,
    maxDepth: overrides.maxDepth ?? V1_HARD_LIMITS.maxDepth,
    maxCanonicalBytes: overrides.maxCanonicalBytes ?? V1_HARD_LIMITS.maxCanonicalBytes,
    maxDirectArguments: overrides.maxDirectArguments ?? V1_HARD_LIMITS.maxDirectArguments,
  };
  for (const key of Object.keys(V1_HARD_LIMITS) as (keyof CanonicalizationLimits)[]) {
    const value = resolved[key];
    const ceiling = V1_HARD_LIMITS[key];
    if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) {
      fail("limit_exceeded", `$limits.${key}`, `must be an integer from 1 through ${ceiling}`);
    }
  }
  return Object.freeze(resolved);
}

export function assert_identifier(value: unknown, path: string): string {
  const identifier = require_string(value, path);
  const byteLength = textEncoder.encode(identifier).byteLength;
  if (byteLength < 1 || byteLength > 128 || !IDENTIFIER_PATTERN.test(identifier)) {
    fail(
      "invalid_identifier",
      path,
      "must be a 1-128 byte lowercase dot-qualified ASCII identifier",
    );
  }
  return identifier;
}

export function assert_policy_id(value: unknown, path: string): string {
  const policyId = require_string(value, path);
  const byteLength = textEncoder.encode(policyId).byteLength;
  if (byteLength < 1 || byteLength > 128 || !POLICY_ID_PATTERN.test(policyId)) {
    fail("invalid_identifier", path, "must match ^[a-z][a-z0-9_.-]{0,127}$ in ASCII");
  }
  return policyId;
}

export function assert_unsigned_decimal(value: unknown, path: string): string {
  const revision = require_string(value, path);
  if (
    !POSITIVE_UNSIGNED_DECIMAL_PATTERN.test(revision) ||
    revision.length > 20 ||
    BigInt(revision) > 18_446_744_073_709_551_615n
  ) {
    fail("invalid_revision", path, "must be canonical decimal in 1..=18446744073709551615");
  }
  return revision;
}

export function assert_canonical_decimal(value: unknown, path: string): string {
  const decimal = require_string(value, path);
  if (decimal === "-0" || !CANONICAL_DECIMAL_PATTERN.test(decimal)) {
    fail("invalid_decimal", path, "must be a normalized fixed-point decimal string");
  }
  return decimal;
}

export function assert_sha256_id(value: unknown, path: string): Sha256Id {
  const digest = require_string(value, path);
  if (!SHA256_PATTERN.test(digest)) {
    fail("invalid_digest", path, "must be sha256: followed by 64 lowercase hexadecimal digits");
  }
  return digest as Sha256Id;
}

interface DecodeState {
  nodes: number;
  readonly limits: Readonly<CanonicalizationLimits>;
}

export function decode_factor_ast(
  input: unknown,
  limits: Readonly<CanonicalizationLimits>,
): FactorAst {
  const state: DecodeState = { nodes: 0, limits };
  return decode_factor_node(input, "$", state, 1);
}

function decode_factor_node(
  input: unknown,
  path: string,
  state: DecodeState,
  depth: number,
): FactorAst {
  if (depth > state.limits.maxDepth) {
    fail("limit_exceeded", path, `AST exceeds ${state.limits.maxDepth} levels`);
  }
  state.nodes += 1;
  if (state.nodes > state.limits.maxNodes) {
    fail("limit_exceeded", path, `AST exceeds ${state.limits.maxNodes} nodes`);
  }

  const object = require_object(input, path);
  const node = require_string(object.node, `${path}.node`);
  switch (node) {
    case "field": {
      require_exact_keys(object, ["node", "field"], path);
      return Object.freeze({
        node,
        field: assert_identifier(object.field, `${path}.field`),
      });
    }
    case "decimal": {
      require_exact_keys(object, ["node", "value"], path);
      return Object.freeze({
        node,
        value: assert_canonical_decimal(object.value, `${path}.value`),
      });
    }
    case "boolean": {
      require_exact_keys(object, ["node", "value"], path);
      return Object.freeze({
        node,
        value: require_boolean(object.value, `${path}.value`),
      });
    }
    case "enum": {
      require_exact_keys(object, ["node", "enum_type", "value"], path);
      return Object.freeze({
        node,
        enum_type: assert_identifier(object.enum_type, `${path}.enum_type`),
        value: assert_identifier(object.value, `${path}.value`),
      });
    }
    case "call": {
      require_exact_keys(object, ["node", "operator", "operator_version", "arguments"], path);
      if (!Array.isArray(object.arguments)) {
        fail("invalid_shape", `${path}.arguments`, "expected an array");
      }
      if (object.arguments.length > state.limits.maxDirectArguments) {
        fail(
          "limit_exceeded",
          `${path}.arguments`,
          `call exceeds ${state.limits.maxDirectArguments} direct arguments`,
        );
      }
      const arguments_ = object.arguments.map((argument, index) =>
        decode_factor_node(argument, `${path}.arguments[${index}]`, state, depth + 1),
      );
      return Object.freeze({
        node,
        operator: assert_identifier(object.operator, `${path}.operator`),
        operator_version: assert_unsigned_decimal(
          object.operator_version,
          `${path}.operator_version`,
        ),
        arguments: Object.freeze(arguments_),
      });
    }
    default:
      fail("invalid_shape", `${path}.node`, `unknown AST node variant ${JSON.stringify(node)}`);
  }
}

export function decode_factor_spec(input: unknown): FactorSpec {
  const object = require_object(input, "$factor_spec");
  require_exact_keys(
    object,
    [
      "schema",
      "expression_id",
      "operator_registry_sha256",
      "direction",
      ...FACTOR_SPEC_POLICY_FIELDS,
    ],
    "$factor_spec",
  );
  if (object.schema !== FACTOR_SPEC_SCHEMA) {
    fail("invalid_shape", "$factor_spec.schema", `must equal ${FACTOR_SPEC_SCHEMA}`);
  }
  if (object.direction !== "higher_is_better" && object.direction !== "lower_is_better") {
    fail("invalid_shape", "$factor_spec.direction", "must be higher_is_better or lower_is_better");
  }

  return Object.freeze({
    schema: FACTOR_SPEC_SCHEMA,
    expression_id: assert_sha256_id(object.expression_id, "$factor_spec.expression_id"),
    operator_registry_sha256: assert_sha256_id(
      object.operator_registry_sha256,
      "$factor_spec.operator_registry_sha256",
    ),
    direction: object.direction,
    universe_policy: decode_policy_ref(object.universe_policy, "$factor_spec.universe_policy"),
    data_policy: decode_policy_ref(object.data_policy, "$factor_spec.data_policy"),
    calendar_policy: decode_policy_ref(object.calendar_policy, "$factor_spec.calendar_policy"),
    preprocess_policy: decode_policy_ref(
      object.preprocess_policy,
      "$factor_spec.preprocess_policy",
    ),
    neutralization_policy: decode_policy_ref(
      object.neutralization_policy,
      "$factor_spec.neutralization_policy",
    ),
    portfolio_policy: decode_policy_ref(object.portfolio_policy, "$factor_spec.portfolio_policy"),
    execution_policy: decode_policy_ref(object.execution_policy, "$factor_spec.execution_policy"),
    cost_policy: decode_policy_ref(object.cost_policy, "$factor_spec.cost_policy"),
    evaluation_policy: decode_policy_ref(
      object.evaluation_policy,
      "$factor_spec.evaluation_policy",
    ),
  });
}

function decode_policy_ref(input: unknown, path: string): PolicyRef {
  const object = require_object(input, path);
  require_exact_keys(object, ["policy_id", "revision", "sha256"], path);
  return Object.freeze({
    policy_id: assert_policy_id(object.policy_id, `${path}.policy_id`),
    revision: assert_unsigned_decimal(object.revision, `${path}.revision`),
    sha256: assert_sha256_id(object.sha256, `${path}.sha256`),
  });
}

function normalize_value_type(input: unknown, path: string): ValueType {
  if (input === "series" || input === "decimal" || input === "boolean") {
    return input;
  }
  const object = require_object(input, path);
  require_exact_keys(object, ["enumType"], path);
  return Object.freeze({ enumType: assert_identifier(object.enumType, `${path}.enumType`) });
}

function normalize_decimal_constraints(input: unknown, path: string): DecimalConstraints {
  const object = require_object(input, path);
  const permitted = ["maxPrecision", "maxScale", "minimum", "maximum"] as const;
  for (const key of Object.keys(object)) {
    if (!permitted.includes(key as (typeof permitted)[number])) {
      fail("invalid_registry", `${path}.${key}`, "unknown decimal constraint");
    }
  }
  for (const key of ["maxPrecision", "maxScale", "minimum", "maximum"] as const) {
    if (!Object.hasOwn(object, key)) {
      fail("invalid_registry", `${path}.${key}`, "missing required constraint");
    }
  }
  const maxPrecision = require_positive_integer(object.maxPrecision, `${path}.maxPrecision`, 4_096);
  const maxScale = require_nonnegative_integer(object.maxScale, `${path}.maxScale`, maxPrecision);
  const minimum = assert_canonical_decimal(object.minimum, `${path}.minimum`);
  const maximum = assert_canonical_decimal(object.maximum, `${path}.maximum`);
  if (compare_decimals(minimum, maximum) > 0) {
    fail("invalid_registry", path, "minimum must not exceed maximum");
  }
  const shapeConstraints = { maxPrecision, maxScale, minimum: undefined, maximum: undefined };
  validate_decimal_constraints(minimum, shapeConstraints, `${path}.minimum`);
  validate_decimal_constraints(maximum, shapeConstraints, `${path}.maximum`);
  return Object.freeze({ maxPrecision, maxScale, minimum, maximum });
}

function normalize_argument_definition(input: unknown, path: string): ArgumentDefinition {
  const object = require_object(input, path);
  const permitted = ["type", "literalOnly", "decimal"] as const;
  for (const key of Object.keys(object)) {
    if (!permitted.includes(key as (typeof permitted)[number])) {
      fail("invalid_registry", `${path}.${key}`, "unknown argument definition field");
    }
  }
  if (!Object.hasOwn(object, "type")) {
    fail("invalid_registry", `${path}.type`, "missing required field");
  }
  const type = normalize_value_type(object.type, `${path}.type`);
  const literalOnly =
    object.literalOnly === undefined
      ? false
      : require_boolean_definition(object.literalOnly, `${path}.literalOnly`);
  const decimal =
    object.decimal === undefined
      ? undefined
      : normalize_decimal_constraints(object.decimal, `${path}.decimal`);
  if (type === "decimal" && decimal === undefined) {
    fail(
      "invalid_registry",
      `${path}.decimal`,
      "decimal arguments must declare precision, scale, and range constraints",
    );
  }
  if (decimal !== undefined && type !== "decimal") {
    fail("invalid_registry", `${path}.decimal`, "constraints require decimal argument type");
  }
  return Object.freeze({ type, literalOnly, decimal });
}

function value_types_equal(left: ValueType, right: ValueType): boolean {
  if (typeof left === "string" || typeof right === "string") {
    return left === right;
  }
  return left.enumType === right.enumType;
}

function argument_definitions_equal(left: ArgumentDefinition, right: ArgumentDefinition): boolean {
  if (
    !value_types_equal(left.type, right.type) ||
    (left.literalOnly ?? false) !== (right.literalOnly ?? false)
  ) {
    return false;
  }
  if (left.decimal === undefined || right.decimal === undefined) {
    return left.decimal === right.decimal;
  }
  return (
    left.decimal.maxPrecision === right.decimal.maxPrecision &&
    left.decimal.maxScale === right.decimal.maxScale &&
    left.decimal.minimum === right.decimal.minimum &&
    left.decimal.maximum === right.decimal.maximum
  );
}

function value_type_label(type: ValueType): string {
  return typeof type === "string" ? type : `enum:${type.enumType}`;
}

function operator_key(operator: string, version: string): string {
  return `${operator}\u0000${version}`;
}

function normalize_field_definition(input: unknown, path: string): FieldDefinition {
  const object = require_object(input, path);
  require_exact_keys(object, ["field", "outputType"], path);
  return Object.freeze({
    field: assert_identifier(object.field, `${path}.field`),
    outputType: normalize_value_type(object.outputType, `${path}.outputType`),
  });
}

function normalize_enum_definition(input: unknown, path: string): EnumDefinition {
  const object = require_object(input, path);
  require_exact_keys(object, ["enumType", "values"], path);
  if (!Array.isArray(object.values) || object.values.length === 0) {
    fail("invalid_registry", `${path}.values`, "must be a non-empty array");
  }
  const values = object.values.map((value, index) =>
    assert_identifier(value, `${path}.values[${index}]`),
  );
  if (new Set(values).size !== values.length) {
    fail("invalid_registry", `${path}.values`, "must not contain duplicate values");
  }
  return Object.freeze({
    enumType: assert_identifier(object.enumType, `${path}.enumType`),
    values: Object.freeze(values),
  });
}

function normalize_operator_definition(
  input: unknown,
  path: string,
): Omit<ResolvedOperatorDefinition, "semanticContract"> {
  const object = require_object(input, path);
  const permitted = [
    "operator",
    "operatorVersion",
    "semanticContractSha256",
    "parameters",
    "variadic",
    "minArguments",
    "maxArguments",
    "outputType",
    "associative",
    "commutative",
  ] as const;
  for (const key of Object.keys(object)) {
    if (!permitted.includes(key as (typeof permitted)[number])) {
      fail("invalid_registry", `${path}.${key}`, "unknown operator definition field");
    }
  }
  for (const key of [
    "operator",
    "operatorVersion",
    "semanticContractSha256",
    "parameters",
    "outputType",
    "associative",
    "commutative",
  ] as const) {
    if (!Object.hasOwn(object, key)) {
      fail("invalid_registry", `${path}.${key}`, "missing required field");
    }
  }
  if (!Array.isArray(object.parameters)) {
    fail("invalid_registry", `${path}.parameters`, "must be an array");
  }
  if (object.parameters.length > V1_HARD_LIMITS.maxDirectArguments) {
    fail("invalid_registry", `${path}.parameters`, "exceeds the v1 direct argument ceiling");
  }

  const parameters = object.parameters.map((parameter, index) =>
    normalize_argument_definition(parameter, `${path}.parameters[${index}]`),
  );
  const variadic =
    object.variadic === undefined
      ? undefined
      : normalize_argument_definition(object.variadic, `${path}.variadic`);
  let minArguments: number;
  let maxArguments: number;
  if (variadic === undefined) {
    minArguments = parameters.length;
    maxArguments = parameters.length;
    if (
      (object.minArguments !== undefined && object.minArguments !== minArguments) ||
      (object.maxArguments !== undefined && object.maxArguments !== maxArguments)
    ) {
      fail("invalid_registry", path, "fixed signatures cannot override their exact arity");
    }
  } else {
    minArguments =
      object.minArguments === undefined
        ? Math.max(parameters.length, 1)
        : require_positive_integer(
            object.minArguments,
            `${path}.minArguments`,
            V1_HARD_LIMITS.maxDirectArguments,
          );
    maxArguments =
      object.maxArguments === undefined
        ? V1_HARD_LIMITS.maxDirectArguments
        : require_positive_integer(
            object.maxArguments,
            `${path}.maxArguments`,
            V1_HARD_LIMITS.maxDirectArguments,
          );
    if (minArguments < parameters.length || maxArguments < minArguments) {
      fail("invalid_registry", path, "variadic arity bounds are inconsistent");
    }
  }

  const outputType = normalize_value_type(object.outputType, `${path}.outputType`);
  const associative = require_boolean_definition(object.associative, `${path}.associative`);
  const commutative = require_boolean_definition(object.commutative, `${path}.commutative`);
  if (associative && variadic === undefined) {
    fail("invalid_registry", path, "associative operators must have a variadic signature");
  }
  if (associative || commutative) {
    const argumentDefinitions = [...parameters];
    if (variadic !== undefined) {
      argumentDefinitions.push(variadic);
    }
    const firstDefinition = argumentDefinitions[0];
    if (
      argumentDefinitions.some((definition) => !value_types_equal(definition.type, outputType)) ||
      (firstDefinition !== undefined &&
        argumentDefinitions.some(
          (definition) => !argument_definitions_equal(definition, firstDefinition),
        ))
    ) {
      fail(
        "invalid_registry",
        path,
        "rewritable operators must have homogeneous argument rules and output types",
      );
    }
  }

  return {
    operator: assert_identifier(object.operator, `${path}.operator`),
    operatorVersion: assert_unsigned_decimal(object.operatorVersion, `${path}.operatorVersion`),
    semanticContractSha256: assert_sha256_id(
      object.semanticContractSha256,
      `${path}.semanticContractSha256`,
    ),
    parameters: Object.freeze(parameters),
    variadic,
    minArguments,
    maxArguments,
    outputType,
    associative,
    commutative,
  };
}

function require_enum_value<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  path: string,
): T[number] {
  const text = require_string(value, path);
  if (!(allowed as readonly string[]).includes(text)) {
    fail("invalid_registry", path, `unsupported semantic policy ${text}`);
  }
  return text as T[number];
}

function write_semantic_contract(contract: OperatorSemanticContract): string {
  return (
    `{"schema":"${OPERATOR_SEMANTIC_CONTRACT_SCHEMA}","operator":"${contract.operator}",` +
    `"operatorVersion":"${contract.operatorVersion}","nullPolicy":"${contract.nullPolicy}",` +
    `"windowPolicy":"${contract.windowPolicy}","tiePolicy":"${contract.tiePolicy}",` +
    `"alignmentPolicy":"${contract.alignmentPolicy}","numericPolicy":"${contract.numericPolicy}"}`
  );
}

export function parse_semantic_contract(bytes: Uint8Array): OperatorSemanticContract {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength > 4_096) {
    fail("invalid_registry", "$semantic_contract", "must be at most 4096 bytes");
  }
  let text: string;
  let parsed: unknown;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    parsed = JSON.parse(text) as unknown;
  } catch {
    fail("invalid_registry", "$semantic_contract", "must be valid UTF-8 JSON");
  }
  const object = require_object(parsed, "$semantic_contract");
  require_exact_keys(
    object,
    [
      "schema",
      "operator",
      "operatorVersion",
      "nullPolicy",
      "windowPolicy",
      "tiePolicy",
      "alignmentPolicy",
      "numericPolicy",
    ],
    "$semantic_contract",
  );
  if (object.schema !== OPERATOR_SEMANTIC_CONTRACT_SCHEMA) {
    fail("invalid_registry", "$semantic_contract.schema", "unsupported semantic schema");
  }
  const contract = Object.freeze({
    schema: OPERATOR_SEMANTIC_CONTRACT_SCHEMA,
    operator: assert_identifier(object.operator, "$semantic_contract.operator"),
    operatorVersion: assert_unsigned_decimal(
      object.operatorVersion,
      "$semantic_contract.operatorVersion",
    ),
    nullPolicy: require_enum_value(
      object.nullPolicy,
      NULL_POLICIES,
      "$semantic_contract.nullPolicy",
    ),
    windowPolicy: require_enum_value(
      object.windowPolicy,
      WINDOW_POLICIES,
      "$semantic_contract.windowPolicy",
    ),
    tiePolicy: require_enum_value(object.tiePolicy, TIE_POLICIES, "$semantic_contract.tiePolicy"),
    alignmentPolicy: require_enum_value(
      object.alignmentPolicy,
      ALIGNMENT_POLICIES,
      "$semantic_contract.alignmentPolicy",
    ),
    numericPolicy: require_enum_value(
      object.numericPolicy,
      NUMERIC_POLICIES,
      "$semantic_contract.numericPolicy",
    ),
  });
  if (write_semantic_contract(contract) !== text) {
    fail("invalid_registry", "$semantic_contract", "bytes are not canonical v1 JSON");
  }
  return contract;
}

export function semantic_contract_sha256(bytes: Uint8Array): Sha256Id {
  const hash = createHash("sha256");
  hash.update(bytes);
  return `sha256:${hash.digest("hex")}` as Sha256Id;
}

export class OperatorPolicyRegistry {
  readonly #fields: ReadonlyMap<string, ValueType>;
  readonly #enums: ReadonlyMap<string, ReadonlySet<string>>;
  readonly #operators: ReadonlyMap<string, ResolvedOperatorDefinition>;
  readonly #canonicalJson: string;
  readonly #sha256: Sha256Id;

  public constructor(snapshot: OperatorRegistrySnapshot, resolver: SemanticContractResolver) {
    const object = require_object(snapshot, "$registry");
    require_exact_keys(object, ["fields", "enums", "operators"], "$registry");
    if (
      !Array.isArray(object.fields) ||
      !Array.isArray(object.enums) ||
      !Array.isArray(object.operators)
    ) {
      fail("invalid_registry", "$registry", "fields, enums, and operators must be arrays");
    }

    const fields = new Map<string, ValueType>();
    object.fields.forEach((input, index) => {
      const definition = normalize_field_definition(input, `$registry.fields[${index}]`);
      if (fields.has(definition.field)) {
        fail("invalid_registry", `$registry.fields[${index}].field`, "duplicate field definition");
      }
      fields.set(definition.field, definition.outputType);
    });

    const enums = new Map<string, ReadonlySet<string>>();
    object.enums.forEach((input, index) => {
      const definition = normalize_enum_definition(input, `$registry.enums[${index}]`);
      if (enums.has(definition.enumType)) {
        fail("invalid_registry", `$registry.enums[${index}].enumType`, "duplicate enum definition");
      }
      enums.set(definition.enumType, new Set(definition.values));
    });

    const operators = new Map<string, ResolvedOperatorDefinition>();
    object.operators.forEach((input, index) => {
      const normalized = normalize_operator_definition(input, `$registry.operators[${index}]`);
      const bytes = resolver(normalized.semanticContractSha256);
      if (bytes === undefined) {
        fail(
          "invalid_registry",
          `$registry.operators[${index}].semanticContractSha256`,
          "semantic contract was not resolved",
        );
      }
      const computed = semantic_contract_sha256(bytes);
      if (computed !== normalized.semanticContractSha256) {
        fail(
          "invalid_registry",
          `$registry.operators[${index}].semanticContractSha256`,
          `semantic contract content digest is ${computed}`,
        );
      }
      const semanticContract = parse_semantic_contract(bytes);
      if (
        semanticContract.operator !== normalized.operator ||
        semanticContract.operatorVersion !== normalized.operatorVersion
      ) {
        fail(
          "invalid_registry",
          `$registry.operators[${index}].semanticContractSha256`,
          "semantic contract operator identity does not match its registry definition",
        );
      }
      const definition = Object.freeze({ ...normalized, semanticContract });
      const key = operator_key(definition.operator, definition.operatorVersion);
      if (operators.has(key)) {
        fail("invalid_registry", `$registry.operators[${index}]`, "duplicate operator version");
      }
      operators.set(key, definition);
    });

    const registeredTypes: ValueType[] = [...fields.values()];
    for (const operator of operators.values()) {
      registeredTypes.push(operator.outputType);
      registeredTypes.push(...operator.parameters.map((parameter) => parameter.type));
      if (operator.variadic !== undefined) {
        registeredTypes.push(operator.variadic.type);
      }
    }
    for (const type of registeredTypes) {
      if (typeof type !== "string" && !enums.has(type.enumType)) {
        fail("invalid_registry", "$registry", `references unknown enum type ${type.enumType}`);
      }
    }

    this.#fields = fields;
    this.#enums = enums;
    this.#operators = operators;
    this.#canonicalJson = write_operator_registry(fields, enums, operators);
    this.#sha256 = hash_operator_registry(new TextEncoder().encode(this.#canonicalJson));
    Object.freeze(this);
  }

  public get canonical_json(): string {
    return this.#canonicalJson;
  }

  public get sha256(): Sha256Id {
    return this.#sha256;
  }

  public resolve_semantic_contract(
    operator: string,
    operatorVersion: string,
  ): OperatorSemanticContract {
    const key = operator_key(
      assert_identifier(operator, "$operator"),
      assert_unsigned_decimal(operatorVersion, "$operatorVersion"),
    );
    const definition = this.#operators.get(key);
    if (definition === undefined) {
      fail("unknown_operator", "$operator", `operator ${operator}@${operatorVersion} is unknown`);
    }
    return definition.semanticContract;
  }

  public to_bytes(): Uint8Array {
    return new TextEncoder().encode(this.#canonicalJson);
  }

  public resolve_field(field: string, path: string): ValueType {
    const type = this.#fields.get(field);
    if (type === undefined) {
      fail("unknown_field", path, `field ${field} is not present in this registry snapshot`);
    }
    return type;
  }

  public resolve_enum(enumType: string, value: string, path: string): ValueType {
    const values = this.#enums.get(enumType);
    if (values === undefined || !values.has(value)) {
      fail("unknown_enum", path, `enum value ${enumType}.${value} is not registered`);
    }
    return Object.freeze({ enumType });
  }

  public resolve_operator(
    operator: string,
    operatorVersion: string,
    path: string,
  ): ResolvedOperatorDefinition {
    const definition = this.#operators.get(operator_key(operator, operatorVersion));
    if (definition === undefined) {
      fail(
        "unknown_operator",
        path,
        `operator ${operator}@${operatorVersion} is not present in this registry snapshot`,
      );
    }
    return definition;
  }

  public validate_arguments(
    definition: ResolvedOperatorDefinition,
    arguments_: readonly ResolvedArgument[],
    path: string,
  ): ValueType {
    if (
      arguments_.length < definition.minArguments ||
      arguments_.length > definition.maxArguments
    ) {
      fail(
        "arity_mismatch",
        path,
        `${definition.operator}@${definition.operatorVersion} expects ${definition.minArguments}-${definition.maxArguments} arguments, received ${arguments_.length}`,
      );
    }
    arguments_.forEach((argument, index) => {
      const expected =
        index < definition.parameters.length ? definition.parameters[index] : definition.variadic;
      if (expected === undefined) {
        fail("arity_mismatch", `${path}[${index}]`, "unexpected argument");
      }
      if (!value_types_equal(argument.type, expected.type)) {
        fail(
          "type_mismatch",
          `${path}[${index}]`,
          `expected ${value_type_label(expected.type)}, received ${value_type_label(argument.type)}`,
        );
      }
      if (expected.literalOnly && !is_typed_literal(argument.ast, expected.type)) {
        fail("type_mismatch", `${path}[${index}]`, "argument must be a literal");
      }
      if (expected.decimal !== undefined && argument.ast.node === "decimal") {
        validate_decimal_constraints(argument.ast.value, expected.decimal, `${path}[${index}]`);
      }
    });
    return definition.outputType;
  }
}

function write_operator_registry(
  fields: ReadonlyMap<string, ValueType>,
  enums: ReadonlyMap<string, ReadonlySet<string>>,
  operators: ReadonlyMap<string, ResolvedOperatorDefinition>,
): string {
  const fieldJson = [...fields.entries()]
    .sort(([left], [right]) => compare_ascii(left, right))
    .map(
      ([field, outputType]) =>
        `{"field":"${field}","outputType":${write_registry_type(outputType)}}`,
    );
  const enumJson = [...enums.entries()]
    .sort(([left], [right]) => compare_ascii(left, right))
    .map(([enumType, values]) => {
      const valueJson = [...values]
        .sort(compare_ascii)
        .map((value) => `"${value}"`)
        .join(",");
      return `{"enumType":"${enumType}","values":[${valueJson}]}`;
    });
  const operatorJson = [...operators.values()]
    .sort((left, right) => {
      const name = compare_ascii(left.operator, right.operator);
      return name === 0
        ? compare_positive_integer(left.operatorVersion, right.operatorVersion)
        : name;
    })
    .map(write_registry_operator);
  return `{"schema":"${OPERATOR_REGISTRY_SCHEMA}","fields":[${fieldJson.join(",")}],"enums":[${enumJson.join(",")}],"operators":[${operatorJson.join(",")}]}`;
}

function write_registry_type(valueType: ValueType): string {
  return typeof valueType === "string" ? `"${valueType}"` : `{"enumType":"${valueType.enumType}"}`;
}

function write_registry_argument(argument: ArgumentDefinition): string {
  let output = `{"type":${write_registry_type(argument.type)}`;
  if (argument.literalOnly === true) {
    output += ',"literalOnly":true';
  }
  if (argument.decimal !== undefined) {
    output +=
      `,"decimal":{"maxPrecision":"${argument.decimal.maxPrecision}",` +
      `"maxScale":"${argument.decimal.maxScale}","minimum":"${argument.decimal.minimum}",` +
      `"maximum":"${argument.decimal.maximum}"}`;
  }
  return `${output}}`;
}

function write_registry_operator(definition: ResolvedOperatorDefinition): string {
  const parameters = definition.parameters.map(write_registry_argument).join(",");
  let output =
    `{"operator":"${definition.operator}","operatorVersion":"${definition.operatorVersion}",` +
    `"semanticContractSha256":"${definition.semanticContractSha256}",` +
    `"parameters":[${parameters}]`;
  if (definition.variadic !== undefined) {
    output +=
      `,"variadic":${write_registry_argument(definition.variadic)}` +
      `,"minArguments":"${definition.minArguments}",` +
      `"maxArguments":"${definition.maxArguments}"`;
  }
  output +=
    `,"outputType":${write_registry_type(definition.outputType)}` +
    `,"associative":${definition.associative},"commutative":${definition.commutative}}`;
  return output;
}

function hash_operator_registry(canonicalBytes: Uint8Array): Sha256Id {
  const hash = createHash("sha256");
  hash.update(OPERATOR_REGISTRY_SCHEMA, "ascii");
  hash.update(Uint8Array.of(0));
  hash.update(canonicalBytes);
  return `sha256:${hash.digest("hex")}` as Sha256Id;
}

function compare_ascii(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function compare_positive_integer(left: string, right: string): number {
  return left.length === right.length ? compare_ascii(left, right) : left.length - right.length;
}

function is_typed_literal(ast: FactorAst, type: ValueType): boolean {
  if (type === "decimal") {
    return ast.node === "decimal";
  }
  if (type === "boolean") {
    return ast.node === "boolean";
  }
  if (typeof type !== "string") {
    return ast.node === "enum" && ast.enum_type === type.enumType;
  }
  return false;
}

function decimal_parts(value: string): {
  readonly negative: boolean;
  readonly integer: string;
  readonly fraction: string;
} {
  const negative = value.startsWith("-");
  const unsigned = negative ? value.slice(1) : value;
  const [integer = "", fraction = ""] = unsigned.split(".");
  return { negative, integer, fraction };
}

function compare_decimals(left: string, right: string): number {
  const leftParts = decimal_parts(left);
  const rightParts = decimal_parts(right);
  const scale = Math.max(leftParts.fraction.length, rightParts.fraction.length);
  const leftMagnitude = BigInt(`${leftParts.integer}${leftParts.fraction.padEnd(scale, "0")}`);
  const rightMagnitude = BigInt(`${rightParts.integer}${rightParts.fraction.padEnd(scale, "0")}`);
  const leftScaled = leftParts.negative ? -leftMagnitude : leftMagnitude;
  const rightScaled = rightParts.negative ? -rightMagnitude : rightMagnitude;
  return leftScaled < rightScaled ? -1 : leftScaled > rightScaled ? 1 : 0;
}

type DecimalValidationConstraints = Pick<DecimalConstraints, "maxPrecision" | "maxScale"> &
  Partial<Pick<DecimalConstraints, "minimum" | "maximum">>;

function validate_decimal_constraints(
  value: string,
  constraints: DecimalValidationConstraints,
  path: string,
): void {
  const parts = decimal_parts(value);
  const precision = parts.integer.length + parts.fraction.length;
  if (precision > constraints.maxPrecision) {
    fail("invalid_decimal", path, `precision exceeds ${constraints.maxPrecision}`);
  }
  if (parts.fraction.length > constraints.maxScale) {
    fail("invalid_decimal", path, `scale exceeds ${constraints.maxScale}`);
  }
  if (constraints.minimum !== undefined && compare_decimals(value, constraints.minimum) < 0) {
    fail("invalid_decimal", path, `value is below minimum ${constraints.minimum}`);
  }
  if (constraints.maximum !== undefined && compare_decimals(value, constraints.maximum) > 0) {
    fail("invalid_decimal", path, `value is above maximum ${constraints.maximum}`);
  }
}
