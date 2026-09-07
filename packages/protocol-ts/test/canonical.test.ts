import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import {
  assertPositiveUnsignedDecimal,
  bindFactorSpec,
  FACTOR_SPEC_POLICY_FIELDS,
  FACTOR_SPEC_SCHEMA,
  type FactorAst,
  FactorDomainError,
  type FactorDomainErrorCode,
  type FactorSpec,
  OperatorPolicyRegistry,
  type OperatorRegistrySnapshot,
  parseCanonicalExpression,
  parseCanonicalFactorSpec,
  parseCanonicalOperatorSemanticContract,
  prepareExpression,
  type Sha256Id,
  semanticContractSha256,
  verifyExpressionIdentity,
  verifyFactorSpecIdentity,
} from "../src/index.js";

interface ConformanceFixture {
  readonly registry: OperatorRegistrySnapshot;
  readonly registry_canonical_utf8: string;
  readonly registry_sha256: Sha256Id;
  readonly registry_operator_vectors: {
    readonly accepted: readonly { readonly name: string; readonly definition: unknown }[];
    readonly rejected: readonly { readonly name: string; readonly definition: unknown }[];
  };
  readonly expression_vectors: readonly {
    readonly name: string;
    readonly input: unknown;
    readonly canonical_utf8: string;
    readonly expression_id: string;
  }[];
  readonly factor_spec_vectors: readonly {
    readonly name: string;
    readonly input: unknown;
    readonly expression_canonical_utf8: string;
    readonly canonical_utf8: string;
    readonly factor_spec_id: string;
  }[];
  readonly rejected_factor_bindings: readonly {
    readonly name: string;
    readonly canonical_expression_utf8: string;
    readonly expression_id: Sha256Id;
  }[];
  readonly rejected_registry_bindings: readonly {
    readonly name: string;
    readonly operator_registry_sha256: Sha256Id;
  }[];
  readonly rejected_factor_spec_canonical_mutations: readonly string[];
  readonly accepted_decimals: readonly string[];
  readonly accepted_positive_integers: readonly string[];
  readonly rejected_decimals: readonly string[];
  readonly rejected_positive_integers: readonly string[];
  readonly rejected_identifiers: readonly string[];
  readonly rejected_canonical_utf8: readonly string[];
  readonly rejected_directions: readonly string[];
  readonly rejected_expressions: readonly {
    readonly name: string;
    readonly input: unknown;
    readonly limit_overrides?: { readonly max_direct_arguments?: number };
  }[];
}

interface SemanticContractFixture {
  readonly accepted: readonly {
    readonly name: string;
    readonly canonical_utf8: string;
    readonly sha256: Sha256Id;
  }[];
  readonly rejected_canonical_utf8: readonly string[];
  readonly deep_nesting: number;
  readonly policy_variants: Readonly<Record<string, readonly string[]>>;
}

const conformanceFixture = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/contracts/factor/v1/canonical_vectors.json", import.meta.url),
    "utf8",
  ),
) as ConformanceFixture;

const semanticContractFixture = JSON.parse(
  readFileSync(
    new URL(
      "../../../fixtures/contracts/factor/v1/operator_semantic_contract_vectors.json",
      import.meta.url,
    ),
    "utf8",
  ),
) as SemanticContractFixture;

const fixtureSemanticContracts = new Map<Sha256Id, Uint8Array>(
  semanticContractFixture.accepted.map((vector) => [
    vector.sha256,
    new TextEncoder().encode(vector.canonical_utf8),
  ]),
);
const fixtureSemanticResolver = (identity: Sha256Id): Uint8Array | undefined =>
  fixtureSemanticContracts.get(identity);

type RawOperatorDefinition = Omit<
  OperatorRegistrySnapshot["operators"][number],
  "semanticContractSha256"
>;

function attachSemanticContract(
  definition: RawOperatorDefinition,
  contracts: Map<Sha256Id, Uint8Array>,
): OperatorRegistrySnapshot["operators"][number] {
  const nullPolicy =
    definition.operator === "rolling.mean"
      ? "ignore_missing"
      : definition.operator === "rank.cross_section"
        ? "preserve_target_ignore_peers"
        : "propagate";
  const windowPolicy =
    definition.operator === "rolling.mean"
      ? "trailing_argument_2_full_window_right_inclusive_constant_preserve"
      : "not_applicable";
  const tiePolicy =
    definition.operator === "rank.cross_section"
      ? "argument_2_average_or_dense_valid_count_constant_midpoint"
      : "not_applicable";
  const alignmentPolicy = ["rolling.mean", "rank.cross_section", "math.abs"].includes(
    definition.operator,
  )
    ? "unary_preserve_timestamp_and_security"
    : "strict_timestamp_and_security";
  const numericPolicy =
    definition.operator === "rank.cross_section"
      ? "ordinal_unit_interval"
      : "binary64_non_finite_to_missing";
  const bytes = new TextEncoder().encode(
    JSON.stringify({
      schema: "loop.operator-semantic-contract/v1",
      operator: definition.operator,
      operatorVersion: definition.operatorVersion,
      nullPolicy,
      windowPolicy,
      tiePolicy,
      alignmentPolicy,
      numericPolicy,
    }),
  );
  const identity = semanticContractSha256(bytes);
  contracts.set(identity, bytes);
  return { ...definition, semanticContractSha256: identity };
}

const localSemanticContracts = new Map<Sha256Id, Uint8Array>();
const localOperator = (definition: RawOperatorDefinition) =>
  attachSemanticContract(definition, localSemanticContracts);
const localSemanticResolver = (identity: Sha256Id): Uint8Array | undefined =>
  localSemanticContracts.get(identity);

function registryForOperatorVector(definition: unknown): OperatorPolicyRegistry {
  const contracts = new Map<Sha256Id, Uint8Array>();
  const complete = attachSemanticContract(definition as RawOperatorDefinition, contracts);
  return new OperatorPolicyRegistry(
    {
      fields: [{ field: "market.close", outputType: "series" }],
      enums: [],
      operators: [complete],
    },
    (identity) => contracts.get(identity),
  );
}

const registry = new OperatorPolicyRegistry(
  {
    fields: [
      { field: "market.close", outputType: "series" },
      { field: "market.high", outputType: "series" },
      { field: "market.low", outputType: "series" },
      { field: "market.open", outputType: "series" },
    ],
    enums: [{ enumType: "rank.method", values: ["average", "dense"] }],
    operators: [
      localOperator({
        operator: "arithmetic.add",
        operatorVersion: "1",
        parameters: [],
        variadic: { type: "series" },
        minArguments: 2,
        maxArguments: 1_024,
        outputType: "series",
        associative: true,
        commutative: true,
      }),
      localOperator({
        operator: "arithmetic.add",
        operatorVersion: "2",
        parameters: [],
        variadic: { type: "series" },
        minArguments: 2,
        maxArguments: 1_024,
        outputType: "series",
        associative: false,
        commutative: false,
      }),
      localOperator({
        operator: "arithmetic.ordered_add",
        operatorVersion: "1",
        parameters: [],
        variadic: { type: "series" },
        minArguments: 2,
        maxArguments: 1_024,
        outputType: "series",
        associative: false,
        commutative: false,
      }),
      localOperator({
        operator: "math.abs",
        operatorVersion: "1",
        parameters: [{ type: "series" }],
        outputType: "series",
        associative: false,
        commutative: false,
      }),
      localOperator({
        operator: "rolling.mean",
        operatorVersion: "1",
        parameters: [
          { type: "series" },
          {
            type: "decimal",
            literalOnly: true,
            decimal: { maxPrecision: 3, maxScale: 0, minimum: "2", maximum: "252" },
          },
        ],
        outputType: "series",
        associative: false,
        commutative: false,
      }),
      localOperator({
        operator: "rank.cross_section",
        operatorVersion: "1",
        parameters: [{ type: "series" }, { type: { enumType: "rank.method" }, literalOnly: true }],
        outputType: "series",
        associative: false,
        commutative: false,
      }),
    ],
  },
  localSemanticResolver,
);

const field = (name: string) => ({ node: "field", field: `market.${name}` }) as const;
const call = (operator: string, version: string, arguments_: readonly unknown[]) => ({
  node: "call",
  operator,
  operator_version: version,
  arguments: arguments_,
});

function expectDomainError(action: () => unknown, code: FactorDomainErrorCode): void {
  try {
    action();
  } catch (error) {
    expect(error).toBeInstanceOf(FactorDomainError);
    expect((error as FactorDomainError).code).toBe(code);
    return;
  }
  throw new Error(`expected FactorDomainError(${code})`);
}

const digest = (fill: number): Sha256Id =>
  `sha256:${fill.toString(16).padStart(64, "0")}` as Sha256Id;
const operatorRegistryDigest = registry.sha256;

function factorSpec(
  expressionId: Sha256Id,
  direction: FactorSpec["direction"] = "higher_is_better",
): FactorSpec {
  return {
    schema: FACTOR_SPEC_SCHEMA,
    expression_id: expressionId,
    operator_registry_sha256: operatorRegistryDigest,
    direction,
    universe_policy: { policy_id: "us_common_stock", revision: "1", sha256: digest(1) },
    data_policy: { policy_id: "pit_market_v1", revision: "1", sha256: digest(2) },
    calendar_policy: { policy_id: "xnys_xnas", revision: "1", sha256: digest(3) },
    preprocess_policy: { policy_id: "cross_section_v1", revision: "1", sha256: digest(4) },
    neutralization_policy: {
      policy_id: "industry_size_beta",
      revision: "1",
      sha256: digest(5),
    },
    portfolio_policy: { policy_id: "decile_long_short", revision: "1", sha256: digest(6) },
    execution_policy: { policy_id: "next_tradable_open", revision: "1", sha256: digest(7) },
    cost_policy: { policy_id: "us_equities_cost_v1", revision: "1", sha256: digest(8) },
    evaluation_policy: { policy_id: "factor_admission_v1", revision: "1", sha256: digest(9) },
  };
}

describe("factor canonicalization v1", () => {
  it("consumes every shared semantic contract byte and negative vector", () => {
    for (const vector of semanticContractFixture.accepted) {
      const bytes = new TextEncoder().encode(vector.canonical_utf8);
      const contract = parseCanonicalOperatorSemanticContract(bytes);
      expect(semanticContractSha256(bytes), vector.name).toBe(vector.sha256);
      expect(contract.operator, vector.name).toBeTruthy();
    }
    for (const [policy, variants] of Object.entries(semanticContractFixture.policy_variants)) {
      for (const variant of variants) {
        const contract = {
          schema: "loop.operator-semantic-contract/v1",
          operator: "fixture.semantic",
          operatorVersion: "1",
          nullPolicy: "not_applicable",
          windowPolicy: "not_applicable",
          tiePolicy: "not_applicable",
          alignmentPolicy: "not_applicable",
          numericPolicy: "not_applicable",
          [policy]: variant,
        };
        expect(
          () =>
            parseCanonicalOperatorSemanticContract(
              new TextEncoder().encode(JSON.stringify(contract)),
            ),
          `${policy}=${variant}`,
        ).not.toThrow();
      }
    }
    for (const canonical of semanticContractFixture.rejected_canonical_utf8) {
      expect(() =>
        parseCanonicalOperatorSemanticContract(new TextEncoder().encode(canonical)),
      ).toThrow(FactorDomainError);
    }
    const deeplyNested = `${"[".repeat(semanticContractFixture.deep_nesting)}0${"]".repeat(
      semanticContractFixture.deep_nesting,
    )}`;
    expect(() =>
      parseCanonicalOperatorSemanticContract(new TextEncoder().encode(deeplyNested)),
    ).toThrow(FactorDomainError);
  });

  it("fails closed when semantic content cannot prove its address and operator binding", () => {
    expect(() => new OperatorPolicyRegistry(conformanceFixture.registry, () => undefined)).toThrow(
      FactorDomainError,
    );

    const first = semanticContractFixture.accepted[0];
    const second = semanticContractFixture.accepted[1];
    if (first === undefined || second === undefined) throw new Error("missing semantic fixtures");
    expect(
      () =>
        new OperatorPolicyRegistry(conformanceFixture.registry, (identity) =>
          identity === first.sha256
            ? new TextEncoder().encode(second.canonical_utf8)
            : fixtureSemanticResolver(identity),
        ),
    ).toThrow(FactorDomainError);

    const mismatched = {
      ...conformanceFixture.registry.operators[0],
      semanticContractSha256: semanticContractFixture.accepted[2]?.sha256,
    };
    expect(
      () =>
        new OperatorPolicyRegistry(
          { fields: [], enums: [], operators: [mismatched] } as OperatorRegistrySnapshot,
          fixtureSemanticResolver,
        ),
    ).toThrow(FactorDomainError);
  });

  it("matches the shared canonical operator registry bytes and identity", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    expect(fixtureRegistry.canonicalJson).toBe(conformanceFixture.registry_canonical_utf8);
    expect(fixtureRegistry.sha256).toBe(conformanceFixture.registry_sha256);
    expect(fixtureRegistry.toBytes()).toEqual(
      new TextEncoder().encode(conformanceFixture.registry_canonical_utf8),
    );
    expect(fixtureRegistry.resolveSemanticContract("rolling.mean", "1").nullPolicy).toBe(
      "ignore_missing",
    );
  });

  it("matches every shared cross-language expression vector byte for byte", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    for (const vector of conformanceFixture.expression_vectors) {
      const canonical = prepareExpression(vector.input, fixtureRegistry);
      expect(canonical.canonicalJson, vector.name).toBe(vector.canonical_utf8);
      expect(canonical.expressionId, vector.name).toBe(vector.expression_id);
      expect(canonical.ast, vector.name).toEqual(JSON.parse(vector.canonical_utf8) as FactorAst);
    }
  });

  it("matches every shared cross-language FactorSpec vector byte for byte", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    for (const vector of conformanceFixture.factor_spec_vectors) {
      const canonical = bindFactorSpec(
        vector.input,
        vector.expression_canonical_utf8,
        fixtureRegistry,
      );
      expect(canonical.canonicalJson, vector.name).toBe(vector.canonical_utf8);
      expect(canonical.factorSpecId, vector.name).toBe(vector.factor_spec_id);
      expect(
        parseCanonicalFactorSpec(
          vector.canonical_utf8,
          vector.factor_spec_id,
          vector.expression_canonical_utf8,
          fixtureRegistry,
        ).factorSpecId,
        vector.name,
      ).toBe(vector.factor_spec_id);
    }
  });

  it("rejects a FactorSpec bound against a different registry snapshot", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    const vector = conformanceFixture.factor_spec_vectors[0];
    if (vector === undefined) throw new Error("missing FactorSpec fixture");
    for (const rejection of conformanceFixture.rejected_registry_bindings) {
      expectDomainError(
        () =>
          bindFactorSpec(
            {
              ...(vector.input as FactorSpec),
              operator_registry_sha256: rejection.operator_registry_sha256,
            },
            vector.expression_canonical_utf8,
            fixtureRegistry,
          ),
        "identity_mismatch",
      );
    }
  });

  it("rejects every shared non-series FactorSpec root", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    const template = conformanceFixture.factor_spec_vectors[0]?.input as FactorSpec;
    for (const vector of conformanceFixture.rejected_factor_bindings) {
      expectDomainError(
        () =>
          bindFactorSpec(
            { ...template, expression_id: vector.expression_id },
            vector.canonical_expression_utf8,
            fixtureRegistry,
          ),
        "type_mismatch",
      );
    }
  });

  it("fails closed on every shared scalar and semantic negative vector", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    for (const value of conformanceFixture.accepted_decimals) {
      expect(
        prepareExpression({ node: "decimal", value }, fixtureRegistry).canonicalJson,
      ).toContain(value);
    }
    for (const value of conformanceFixture.accepted_positive_integers) {
      expect(assertPositiveUnsignedDecimal(value, "$fixture"), value).toBe(value);
    }
    for (const value of conformanceFixture.rejected_decimals) {
      expect(() => prepareExpression({ node: "decimal", value }, fixtureRegistry)).toThrow(
        FactorDomainError,
      );
    }
    for (const value of conformanceFixture.rejected_positive_integers) {
      expect(() => assertPositiveUnsignedDecimal(value, "$fixture"), value).toThrow(
        FactorDomainError,
      );
    }
    for (const value of conformanceFixture.rejected_identifiers) {
      expect(() => prepareExpression({ node: "field", field: value }, fixtureRegistry)).toThrow(
        FactorDomainError,
      );
    }
    for (const vector of conformanceFixture.rejected_expressions) {
      expect(
        () =>
          prepareExpression(vector.input, fixtureRegistry, {
            maxDirectArguments: vector.limit_overrides?.max_direct_arguments,
          }),
        vector.name,
      ).toThrow(FactorDomainError);
    }
    for (const canonicalUtf8 of conformanceFixture.rejected_canonical_utf8) {
      expect(() => parseCanonicalExpression(canonicalUtf8, fixtureRegistry)).toThrow(
        FactorDomainError,
      );
    }
  });

  it("uses the shared variadic registry boundary and scalar-type rules", () => {
    for (const vector of conformanceFixture.registry_operator_vectors.accepted) {
      expect(() => registryForOperatorVector(vector.definition), vector.name).not.toThrow();
    }
    for (const vector of conformanceFixture.registry_operator_vectors.rejected) {
      expect(() => registryForOperatorVector(vector.definition), vector.name).toThrow(
        FactorDomainError,
      );
    }
  });

  it("emits fixed canonical bytes and domain-separated expression identity", () => {
    const canonical = prepareExpression(
      call("arithmetic.add", "1", [field("open"), field("close")]),
      registry,
    );

    expect(canonical.canonicalJson).toBe(
      '{"node":"call","operator":"arithmetic.add","operator_version":"1","arguments":[{"node":"field","field":"market.close"},{"node":"field","field":"market.open"}]}',
    );
    expect(canonical.expressionId).toBe(
      "sha256:4a65ce89092ec916ddd9c34d8d9b10d77321d3522c1c0b543c93fb38d42e228d",
    );
    expect(new TextDecoder().decode(canonical.toBytes())).toBe(canonical.canonicalJson);
    expect(Object.isFrozen(canonical.ast)).toBe(true);
  });

  it("rejects non-canonical decimals and every unsafe identifier family", () => {
    for (const value of ["0", "1", "-1", "0.5", "-0.5", "10.25", "0.0001"]) {
      expect(prepareExpression({ node: "decimal", value }, registry).canonicalJson).toContain(
        value,
      );
    }
    for (const value of [
      "",
      "+1",
      "-0",
      "00",
      "01",
      ".5",
      "1.",
      "1.0",
      "1.20",
      "1e3",
      "NaN",
      "Infinity",
    ]) {
      expectDomainError(
        () => prepareExpression({ node: "decimal", value }, registry),
        "invalid_decimal",
      );
    }
    for (const unsafe of [
      "Market.close",
      "2market.close",
      "market..close",
      "market/close",
      "market%2eclose",
      "market.closé",
      `a.${"b".repeat(127)}`,
    ]) {
      expectDomainError(
        () => prepareExpression({ node: "field", field: unsafe }, registry),
        "invalid_identifier",
      );
    }
  });

  it("applies only exact-version associative and commutative registry policies", () => {
    const nested = call("arithmetic.add", "1", [
      field("open"),
      call("arithmetic.add", "1", [field("high"), field("close")]),
    ]);
    const flatPermutation = call("arithmetic.add", "1", [
      field("close"),
      field("open"),
      field("high"),
    ]);
    expect(prepareExpression(nested, registry).expressionId).toBe(
      prepareExpression(flatPermutation, registry).expressionId,
    );

    const orderedLeft = call("arithmetic.ordered_add", "1", [field("open"), field("close")]);
    const orderedRight = call("arithmetic.ordered_add", "1", [field("close"), field("open")]);
    expect(prepareExpression(orderedLeft, registry).expressionId).not.toBe(
      prepareExpression(orderedRight, registry).expressionId,
    );

    const versionTwoNested = call("arithmetic.add", "2", [
      field("open"),
      call("arithmetic.add", "2", [field("high"), field("close")]),
    ]);
    const versionTwoFlat = call("arithmetic.add", "2", [
      field("open"),
      field("high"),
      field("close"),
    ]);
    expect(prepareExpression(versionTwoNested, registry).expressionId).not.toBe(
      prepareExpression(versionTwoFlat, registry).expressionId,
    );
    expect(prepareExpression(orderedLeft, registry).expressionId).not.toBe(
      prepareExpression(call("arithmetic.add", "2", [field("open"), field("close")]), registry)
        .expressionId,
    );
  });

  it("resolves signatures, literal constraints, enum domains, and unknown symbols", () => {
    expect(
      prepareExpression(
        call("rolling.mean", "1", [field("close"), { node: "decimal", value: "20" }]),
        registry,
      ).expressionId,
    ).toMatch(/^sha256:[0-9a-f]{64}$/);
    expectDomainError(
      () =>
        prepareExpression(
          call("rolling.mean", "1", [field("close"), { node: "decimal", value: "1" }]),
          registry,
        ),
      "invalid_decimal",
    );
    expectDomainError(
      () => prepareExpression(call("rolling.mean", "1", [field("close"), field("open")]), registry),
      "type_mismatch",
    );
    expectDomainError(
      () => prepareExpression(call("rolling.mean", "9", [field("close")]), registry),
      "unknown_operator",
    );
    expectDomainError(
      () => prepareExpression({ node: "field", field: "market.volume" }, registry),
      "unknown_field",
    );
    expectDomainError(
      () =>
        prepareExpression({ node: "enum", enum_type: "rank.method", value: "ordinal" }, registry),
      "unknown_enum",
    );
  });

  it("reparses canonical bytes and rejects alternate JSON spellings or identity mismatches", () => {
    const canonical = prepareExpression(field("close"), registry);
    expect(parseCanonicalExpression(canonical.toBytes(), registry).ast).toEqual(canonical.ast);
    expect(
      verifyExpressionIdentity(canonical.expressionId, canonical.toBytes(), registry).expressionId,
    ).toBe(canonical.expressionId);

    expectDomainError(
      () => parseCanonicalExpression(' {"node":"field","field":"market.close"}', registry),
      "non_canonical",
    );
    expectDomainError(
      () => parseCanonicalExpression('{"field":"market.close","node":"field"}', registry),
      "non_canonical",
    );
    expectDomainError(
      () =>
        parseCanonicalExpression(
          '{"node":"field","field":"market.close","field":"market.close"}',
          registry,
        ),
      "non_canonical",
    );
    expectDomainError(
      () => verifyExpressionIdentity(digest(99), canonical.toBytes(), registry),
      "identity_mismatch",
    );
  });

  it("binds direction and every one of the nine policy digests into FactorSpec identity", () => {
    const expression = prepareExpression(field("close"), registry);
    const baselineSpec = factorSpec(expression.expressionId);
    const baseline = bindFactorSpec(baselineSpec, expression.toBytes(), registry);
    expect(baseline.factorSpecId).toMatch(/^sha256:[0-9a-f]{64}$/);
    expect(
      bindFactorSpec(
        factorSpec(expression.expressionId, "lower_is_better"),
        expression.toBytes(),
        registry,
      ).factorSpecId,
    ).not.toBe(baseline.factorSpecId);

    FACTOR_SPEC_POLICY_FIELDS.forEach((fieldName, index) => {
      const changed: FactorSpec = {
        ...baselineSpec,
        [fieldName]: { ...baselineSpec[fieldName], sha256: digest(100 + index) },
      };
      expect(bindFactorSpec(changed, expression.toBytes(), registry).factorSpecId).not.toBe(
        baseline.factorSpecId,
      );
    });

    for (const direction of conformanceFixture.rejected_directions) {
      expectDomainError(
        () => bindFactorSpec({ ...baselineSpec, direction }, expression.toBytes(), registry),
        "invalid_shape",
      );
    }
  });

  it("uses the exact FactorSpec field order and verifies stored bytes on read", () => {
    const expression = prepareExpression(field("close"), registry);
    const canonical = bindFactorSpec(
      factorSpec(expression.expressionId),
      expression.toBytes(),
      registry,
    );
    expect(canonical.factorSpecId).toMatch(/^sha256:[0-9a-f]{64}$/);
    expect(canonical.canonicalJson).toBe(
      `{"schema":"loop.factor-spec/v1","expression_id":"${expression.expressionId}","operator_registry_sha256":"${operatorRegistryDigest}","direction":"higher_is_better","universe_policy":{"policy_id":"us_common_stock","revision":"1","sha256":"${digest(1)}"},"data_policy":{"policy_id":"pit_market_v1","revision":"1","sha256":"${digest(2)}"},"calendar_policy":{"policy_id":"xnys_xnas","revision":"1","sha256":"${digest(3)}"},"preprocess_policy":{"policy_id":"cross_section_v1","revision":"1","sha256":"${digest(4)}"},"neutralization_policy":{"policy_id":"industry_size_beta","revision":"1","sha256":"${digest(5)}"},"portfolio_policy":{"policy_id":"decile_long_short","revision":"1","sha256":"${digest(6)}"},"execution_policy":{"policy_id":"next_tradable_open","revision":"1","sha256":"${digest(7)}"},"cost_policy":{"policy_id":"us_equities_cost_v1","revision":"1","sha256":"${digest(8)}"},"evaluation_policy":{"policy_id":"factor_admission_v1","revision":"1","sha256":"${digest(9)}"}}`,
    );
    expect(
      parseCanonicalFactorSpec(
        canonical.toBytes(),
        canonical.factorSpecId,
        expression.toBytes(),
        registry,
      ).spec,
    ).toEqual(canonical.spec);
    expect(
      verifyFactorSpecIdentity(
        canonical.factorSpecId,
        canonical.toBytes(),
        expression.toBytes(),
        registry,
      ).factorSpecId,
    ).toBe(canonical.factorSpecId);
    expectDomainError(
      () =>
        parseCanonicalFactorSpec(
          ` ${canonical.canonicalJson}`,
          canonical.factorSpecId,
          expression.toBytes(),
          registry,
        ),
      "non_canonical",
    );
    expectDomainError(
      () => bindFactorSpec(factorSpec(digest(99)), expression.toBytes(), registry),
      "identity_mismatch",
    );
  });

  it("rejects every shared malformed canonical FactorSpec form", () => {
    const fixtureRegistry = new OperatorPolicyRegistry(
      conformanceFixture.registry,
      fixtureSemanticResolver,
    );
    const vector = conformanceFixture.factor_spec_vectors[0];
    if (vector === undefined) throw new Error("missing FactorSpec fixture");
    for (const mutation of conformanceFixture.rejected_factor_spec_canonical_mutations) {
      const malformed = mutateFactorSpec(vector.canonical_utf8, mutation);
      expect(
        () =>
          parseCanonicalFactorSpec(
            malformed,
            vector.factor_spec_id,
            vector.expression_canonical_utf8,
            fixtureRegistry,
          ),
        mutation,
      ).toThrow(FactorDomainError);
    }
  });

  it("enforces node, depth, direct-argument, byte, and hard limit ceilings", () => {
    const nested = call("math.abs", "1", [call("math.abs", "1", [field("close")])]);
    expectDomainError(() => prepareExpression(nested, registry, { maxDepth: 2 }), "limit_exceeded");
    expectDomainError(() => prepareExpression(nested, registry, { maxNodes: 2 }), "limit_exceeded");
    expectDomainError(
      () =>
        prepareExpression(
          call("arithmetic.add", "1", [field("close"), field("open"), field("high")]),
          registry,
          { maxDirectArguments: 2 },
        ),
      "limit_exceeded",
    );
    expectDomainError(
      () => prepareExpression(field("close"), registry, { maxCanonicalBytes: 20 }),
      "limit_exceeded",
    );
    expectDomainError(
      () => prepareExpression(field("close"), registry, { maxDepth: 65 }),
      "limit_exceeded",
    );
    expectDomainError(
      () => prepareExpression(field("close"), registry, { unexpected: 1 } as never),
      "limit_exceeded",
    );
  });

  it("rejects hostile nesting before JavaScript exhausts its call stack", () => {
    let nested: FactorAst = field("close");
    for (let depth = 0; depth < 10_000; depth += 1) {
      nested = call("math.abs", "1", [nested]);
    }

    expectDomainError(() => prepareExpression(nested, registry), "limit_exceeded");
  });
});

function mutateFactorSpec(canonical: string, mutation: string): string {
  switch (mutation) {
    case "leading_whitespace":
      return ` ${canonical}`;
    case "reordered_top_level_fields":
      return canonical
        .replace('{"schema":"loop.factor-spec/v1","expression_id":"', '{"expression_id":"')
        .replace(
          '","operator_registry_sha256":"',
          '","schema":"loop.factor-spec/v1","operator_registry_sha256":"',
        );
    case "duplicate_direction":
      return canonical.replace(
        '"direction":"higher_is_better",',
        '"direction":"higher_is_better","direction":"higher_is_better",',
      );
    case "unknown_top_level_field":
      return `${canonical.slice(0, -1)},"unknown":true}`;
    case "missing_evaluation_policy":
      return `${canonical.slice(0, canonical.indexOf(',"evaluation_policy":'))}}`;
    case "reordered_policy_fields":
      return canonical.replace(
        '"universe_policy":{"policy_id":"us_common_stock","revision":"1",',
        '"universe_policy":{"revision":"1","policy_id":"us_common_stock",',
      );
    case "duplicate_policy_field":
      return canonical.replace(
        '"universe_policy":{"policy_id":"us_common_stock",',
        '"universe_policy":{"policy_id":"us_common_stock","policy_id":"us_common_stock",',
      );
    case "unknown_policy_field":
      return canonical.replace(
        '"sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000001"}',
        '"sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000001","unknown":true}',
      );
    case "missing_policy_field":
      return canonical.replace(
        ',"sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000001"',
        "",
      );
    case "wrong_schema":
      return canonical.replace("loop.factor-spec/v1", "loop.factor-spec/v2");
    case "auto_direction":
      return canonical.replace("higher_is_better", "auto");
    default:
      throw new Error(`unknown FactorSpec mutation: ${mutation}`);
  }
}
