from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest

from loop_protocol import (
    ArgumentDefinition,
    BooleanNode,
    CallNode,
    CanonicalizationError,
    DecimalConstraints,
    DecimalNode,
    EnumDefinition,
    EnumNode,
    EnumValueType,
    FactorDirection,
    FactorSpec,
    FieldDefinition,
    FieldNode,
    OperatorDefinition,
    OperatorRegistry,
    PolicyReference,
    ScalarValueType,
    bind_factor_spec,
    canonicalize_expression,
    parse_canonical_ast,
    parse_canonical_factor_spec,
    parse_canonical_operator_semantic_contract,
    semantic_contract_sha256,
)

FIXTURE_PATH = (
    Path(__file__).resolve().parents[3]
    / "fixtures"
    / "contracts"
    / "factor"
    / "v1"
    / "canonical_vectors.json"
)
SEMANTIC_FIXTURE_PATH = FIXTURE_PATH.with_name("operator_semantic_contract_vectors.json")


def _fixture() -> dict[str, Any]:
    with FIXTURE_PATH.open(encoding="utf-8") as stream:
        return json.load(stream)  # type: ignore[no-any-return]


def _semantic_fixture() -> dict[str, Any]:
    with SEMANTIC_FIXTURE_PATH.open(encoding="utf-8") as stream:
        return json.load(stream)  # type: ignore[no-any-return]


def _value_type(raw: str | dict[str, str]) -> ScalarValueType | EnumValueType:
    if isinstance(raw, str):
        return ScalarValueType(raw)
    return EnumValueType(raw["enumType"])


def _argument(raw: dict[str, Any]) -> ArgumentDefinition:
    constraints = raw.get("decimal")
    decimal = (
        DecimalConstraints(
            constraints["maxPrecision"],
            constraints["maxScale"],
            constraints["minimum"],
            constraints["maximum"],
        )
        if constraints is not None
        else None
    )
    return ArgumentDefinition(
        _value_type(raw["type"]),
        literal_only=raw.get("literalOnly", False),
        decimal=decimal,
    )


def _registry(
    raw: dict[str, Any], additional_contracts: dict[str, bytes] | None = None
) -> OperatorRegistry:
    fields = tuple(
        FieldDefinition(item["field"], _value_type(item["outputType"])) for item in raw["fields"]
    )
    enums = tuple(EnumDefinition(item["enumType"], tuple(item["values"])) for item in raw["enums"])
    operators = tuple(
        OperatorDefinition(
            item["operator"],
            item["operatorVersion"],
            tuple(_argument(argument) for argument in item["parameters"]),
            _value_type(item["outputType"]),
            item["semanticContractSha256"],
            variadic=_argument(item["variadic"]) if "variadic" in item else None,
            minimum_arguments=item.get("minArguments"),
            maximum_arguments=item.get("maxArguments"),
            associative=item["associative"],
            commutative=item["commutative"],
        )
        for item in raw["operators"]
    )
    contracts = {
        item["sha256"]: item["canonical_utf8"].encode("utf-8")
        for item in _semantic_fixture()["accepted"]
    }
    if additional_contracts is not None:
        contracts.update(additional_contracts)
    return OperatorRegistry(
        fields=fields,
        enums=enums,
        operators=operators,
        semantic_contract_resolver=contracts.get,
    )


def _node(raw: dict[str, Any]) -> FieldNode | DecimalNode | BooleanNode | EnumNode | CallNode:
    match raw["node"]:
        case "field":
            return FieldNode(raw["field"])
        case "decimal":
            return DecimalNode(raw["value"])
        case "boolean":
            return BooleanNode(raw["value"])
        case "enum":
            return EnumNode(raw["enum_type"], raw["value"])
        case "call":
            return CallNode(
                raw["operator"],
                raw["operator_version"],
                tuple(_node(argument) for argument in raw["arguments"]),
            )
        case unknown:
            raise AssertionError(f"unknown fixture node: {unknown}")


def _policy(raw: dict[str, str]) -> PolicyReference:
    return PolicyReference(raw["policy_id"], raw["revision"], raw["sha256"])


def _factor_spec(raw: dict[str, Any]) -> FactorSpec:
    return FactorSpec(
        expression_id=raw["expression_id"],
        operator_registry_sha256=raw["operator_registry_sha256"],
        direction=FactorDirection(raw["direction"]),
        universe_policy=_policy(raw["universe_policy"]),
        data_policy=_policy(raw["data_policy"]),
        calendar_policy=_policy(raw["calendar_policy"]),
        preprocess_policy=_policy(raw["preprocess_policy"]),
        neutralization_policy=_policy(raw["neutralization_policy"]),
        portfolio_policy=_policy(raw["portfolio_policy"]),
        execution_policy=_policy(raw["execution_policy"]),
        cost_policy=_policy(raw["cost_policy"]),
        evaluation_policy=_policy(raw["evaluation_policy"]),
    )


def test_shared_semantic_contract_vectors_are_exact_and_fail_closed() -> None:
    fixture = _semantic_fixture()
    for vector in fixture["accepted"]:
        canonical = vector["canonical_utf8"].encode("utf-8")
        contract = parse_canonical_operator_semantic_contract(canonical)
        assert semantic_contract_sha256(canonical) == vector["sha256"], vector["name"]
        assert contract.operator
    for policy, variants in fixture["policy_variants"].items():
        for variant in variants:
            contract = {
                "schema": "loop.operator-semantic-contract/v1",
                "operator": "fixture.semantic",
                "operatorVersion": "1",
                "nullPolicy": "not_applicable",
                "windowPolicy": "not_applicable",
                "tiePolicy": "not_applicable",
                "alignmentPolicy": "not_applicable",
                "numericPolicy": "not_applicable",
            }
            contract[policy] = variant
            canonical = json.dumps(contract, separators=(",", ":")).encode()
            parse_canonical_operator_semantic_contract(canonical)
    for canonical_text in fixture["rejected_canonical_utf8"]:
        with pytest.raises(CanonicalizationError):
            parse_canonical_operator_semantic_contract(canonical_text.encode("utf-8"))

    nesting = fixture["deep_nesting"]
    deeply_nested = ("[" * nesting + "0" + "]" * nesting).encode()
    with pytest.raises(CanonicalizationError):
        parse_canonical_operator_semantic_contract(deeply_nested)


def test_registry_rejects_unresolved_misaddressed_and_misbound_semantics() -> None:
    fixture = _fixture()
    semantic_fixture = _semantic_fixture()
    operators = fixture["registry"]["operators"]
    first = operators[0]
    definition = OperatorDefinition(
        first["operator"],
        first["operatorVersion"],
        (),
        ScalarValueType.SERIES,
        first["semanticContractSha256"],
        variadic=ArgumentDefinition(ScalarValueType.SERIES),
        minimum_arguments=2,
        maximum_arguments=1_024,
        associative=True,
        commutative=True,
    )
    with pytest.raises(CanonicalizationError, match="not resolved"):
        OperatorRegistry(
            fields=(), enums=(), operators=(definition,), semantic_contract_resolver=lambda _: None
        )

    wrong_bytes = semantic_fixture["accepted"][1]["canonical_utf8"].encode()
    with pytest.raises(CanonicalizationError, match="digest"):
        OperatorRegistry(
            fields=(),
            enums=(),
            operators=(definition,),
            semantic_contract_resolver=lambda _: wrong_bytes,
        )

    subtract = semantic_fixture["accepted"][2]
    misbound = replace(definition, semantic_contract_sha256=subtract["sha256"])
    with pytest.raises(CanonicalizationError, match="operator identity"):
        OperatorRegistry(
            fields=(),
            enums=(),
            operators=(misbound,),
            semantic_contract_resolver=lambda _: subtract["canonical_utf8"].encode(),
        )


def test_registry_rejects_mutable_or_invalid_nested_operator_definitions() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    identity_before = registry.sha256
    definition_before = registry.require_operator("rolling.mean", "1")
    semantic_sha256 = fixture["registry"]["operators"][0]["semanticContractSha256"]
    series = ArgumentDefinition(ScalarValueType.SERIES)

    mutable_parameters = [series]
    with pytest.raises(CanonicalizationError, match="immutable tuple"):
        OperatorDefinition(
            "fixture.mutable",
            "1",
            mutable_parameters,
            ScalarValueType.SERIES,
            semantic_sha256,
        )
    mutable_parameters.append(series)

    mutable_enum_values = ["average", "dense"]
    with pytest.raises(CanonicalizationError, match="immutable tuple"):
        EnumDefinition("fixture.mutable_enum", mutable_enum_values)
    mutable_enum_values.append("first")

    with pytest.raises(CanonicalizationError, match="invalid definition"):
        OperatorDefinition(
            "fixture.invalid_parameter",
            "1",
            ("not-an-argument",),
            ScalarValueType.SERIES,
            semantic_sha256,
        )
    with pytest.raises(CanonicalizationError, match="variadic definition"):
        OperatorDefinition(
            "fixture.invalid_variadic",
            "1",
            (),
            ScalarValueType.SERIES,
            semantic_sha256,
            variadic=[series],
        )
    with pytest.raises(CanonicalizationError, match="decimal constraints"):
        ArgumentDefinition(
            ScalarValueType.DECIMAL,
            literal_only=True,
            decimal={"minimum": "1"},
        )

    assert isinstance(definition_before.parameters, tuple)
    with pytest.raises(AttributeError):
        definition_before.parameters.append(series)
    assert registry.sha256 == identity_before
    assert registry.require_operator("rolling.mean", "1") == definition_before


def test_call_node_rejects_mutable_or_invalid_argument_containers() -> None:
    close = FieldNode("market.close")
    mutable_arguments = [close]
    with pytest.raises(CanonicalizationError, match="immutable tuple"):
        CallNode("fixture.call", "1", mutable_arguments)
    mutable_arguments.append(FieldNode("market.open"))

    with pytest.raises(CanonicalizationError, match="invalid AST node"):
        CallNode("fixture.call", "1", (close, "not-an-ast-node"))


def test_shared_expression_vectors_are_byte_and_id_exact() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    for vector in fixture["expression_vectors"]:
        result = canonicalize_expression(_node(vector["input"]), registry)
        assert result.canonical_bytes == vector["canonical_utf8"].encode("utf-8"), vector["name"]
        assert result.expression_id == vector["expression_id"], vector["name"]
        assert result.ast == _node(json.loads(vector["canonical_utf8"])), vector["name"]


def test_shared_registry_boundary_and_scalar_type_vectors() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    assert registry.canonical_bytes == fixture["registry_canonical_utf8"].encode("ascii")
    assert registry.sha256 == fixture["registry_sha256"]
    assert registry.require_semantic_contract("rolling.mean", "1").null_policy.value == (
        "ignore_missing"
    )
    identity_before = registry.sha256
    operators = registry._operators
    with pytest.raises(TypeError):
        operators[("rolling.mean", "1")] = None
    with pytest.raises(AttributeError):
        registry._operators = {}
    with pytest.raises(AttributeError):
        registry._sha256 = "sha256:" + "f" * 64
    with pytest.raises((AttributeError, TypeError)):
        registry.injected_state = True
    assert registry.sha256 == identity_before
    assert registry.require_operator("rolling.mean", "1").operator == "rolling.mean"

    def snapshot(definition: dict[str, Any]) -> tuple[dict[str, Any], dict[str, bytes]]:
        contract = {
            "schema": "loop.operator-semantic-contract/v1",
            "operator": definition["operator"],
            "operatorVersion": definition["operatorVersion"],
            "nullPolicy": "propagate",
            "windowPolicy": "not_applicable",
            "tiePolicy": "not_applicable",
            "alignmentPolicy": "strict_timestamp_and_security",
            "numericPolicy": "binary64_non_finite_to_missing",
        }
        contract_bytes = json.dumps(contract, separators=(",", ":")).encode()
        identity = semantic_contract_sha256(contract_bytes)
        complete = {**definition, "semanticContractSha256": identity}
        return (
            {
                "fields": [{"field": "market.close", "outputType": "series"}],
                "enums": [],
                "operators": [complete],
            },
            {identity: contract_bytes},
        )

    for vector in fixture["registry_operator_vectors"]["accepted"]:
        raw, contracts = snapshot(vector["definition"])
        _registry(raw, contracts)
    for vector in fixture["registry_operator_vectors"]["rejected"]:
        with pytest.raises(CanonicalizationError):
            raw, contracts = snapshot(vector["definition"])
            _registry(raw, contracts)


def test_shared_factor_spec_vector_binds_registry_and_all_policies() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    vector = fixture["factor_spec_vectors"][0]
    spec = _factor_spec(vector["input"])
    expression_bytes = vector["expression_canonical_utf8"].encode("utf-8")
    result = bind_factor_spec(spec, expression_bytes, registry)
    assert result.canonical_bytes == vector["canonical_utf8"].encode("utf-8")
    assert result.factor_spec_id == vector["factor_spec_id"]
    assert (
        parse_canonical_factor_spec(
            result.canonical_bytes,
            result.factor_spec_id,
            expression_bytes,
            registry,
        )
        == result
    )

    for rejection in fixture["rejected_registry_bindings"]:
        with pytest.raises(CanonicalizationError, match="resolved registry snapshot"):
            bind_factor_spec(
                replace(
                    spec,
                    operator_registry_sha256=rejection["operator_registry_sha256"],
                ),
                expression_bytes,
                registry,
            )


def test_shared_non_series_roots_cannot_bind_factor_specs() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    template = _factor_spec(fixture["factor_spec_vectors"][0]["input"])
    for vector in fixture["rejected_factor_bindings"]:
        draft = replace(template, expression_id=vector["expression_id"])
        with pytest.raises(CanonicalizationError, match="root must resolve to series"):
            bind_factor_spec(
                draft,
                vector["canonical_expression_utf8"].encode("utf-8"),
                registry,
            )


def test_shared_malformed_canonical_factor_specs_fail_closed() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    vector = fixture["factor_spec_vectors"][0]
    expression_bytes = vector["expression_canonical_utf8"].encode("utf-8")
    for mutation in fixture["rejected_factor_spec_canonical_mutations"]:
        malformed = _mutate_factor_spec(vector["canonical_utf8"], mutation).encode("utf-8")
        with pytest.raises(CanonicalizationError):
            parse_canonical_factor_spec(
                malformed,
                vector["factor_spec_id"],
                expression_bytes,
                registry,
            )


def _mutate_factor_spec(canonical: str, mutation: str) -> str:
    if mutation == "leading_whitespace":
        return f" {canonical}"
    if mutation == "reordered_top_level_fields":
        return canonical.replace(
            '{"schema":"loop.factor-spec/v1","expression_id":"',
            '{"expression_id":"',
            1,
        ).replace(
            '","operator_registry_sha256":"',
            '","schema":"loop.factor-spec/v1","operator_registry_sha256":"',
            1,
        )
    if mutation == "duplicate_direction":
        return canonical.replace(
            '"direction":"higher_is_better",',
            '"direction":"higher_is_better","direction":"higher_is_better",',
            1,
        )
    if mutation == "unknown_top_level_field":
        return f'{canonical[:-1]},"unknown":true}}'
    if mutation == "missing_evaluation_policy":
        return f"{canonical[: canonical.index(',"evaluation_policy":')]}}}"
    if mutation == "reordered_policy_fields":
        return canonical.replace(
            '"universe_policy":{"policy_id":"us_common_stock","revision":"1",',
            '"universe_policy":{"revision":"1","policy_id":"us_common_stock",',
            1,
        )
    if mutation == "duplicate_policy_field":
        return canonical.replace(
            '"universe_policy":{"policy_id":"us_common_stock",',
            '"universe_policy":{"policy_id":"us_common_stock","policy_id":"us_common_stock",',
            1,
        )
    if mutation == "unknown_policy_field":
        return canonical.replace(
            '"sha256":"sha256:' + "0" * 63 + '1"}',
            '"sha256":"sha256:' + "0" * 63 + '1","unknown":true}',
            1,
        )
    if mutation == "missing_policy_field":
        return canonical.replace(
            ',"sha256":"sha256:' + "0" * 63 + '1"',
            "",
            1,
        )
    if mutation == "wrong_schema":
        return canonical.replace("loop.factor-spec/v1", "loop.factor-spec/v2", 1)
    if mutation == "auto_direction":
        return canonical.replace("higher_is_better", "auto", 1)
    raise AssertionError(f"unknown FactorSpec mutation: {mutation}")


def test_shared_scalar_and_semantic_negative_vectors_fail_closed() -> None:
    fixture = _fixture()
    registry = _registry(fixture["registry"])
    for value in fixture["accepted_decimals"]:
        canonicalize_expression(DecimalNode(value), registry)
    for value in fixture["accepted_positive_integers"]:
        assert PolicyReference("bounded_revision", value, "sha256:" + "0" * 64).revision == value
    for value in fixture["rejected_decimals"]:
        with pytest.raises(CanonicalizationError):
            canonicalize_expression(DecimalNode(value), registry)
    for value in fixture["rejected_positive_integers"]:
        with pytest.raises(CanonicalizationError):
            PolicyReference("bounded_revision", value, "sha256:" + "0" * 64)
    for value in fixture["rejected_identifiers"]:
        with pytest.raises(CanonicalizationError):
            canonicalize_expression(FieldNode(value), registry)
    for vector in fixture["rejected_expressions"]:
        with pytest.raises(CanonicalizationError):
            canonicalize_expression(_node(vector["input"]), registry, vector.get("limit_overrides"))
    for canonical_utf8 in fixture["rejected_canonical_utf8"]:
        with pytest.raises(CanonicalizationError):
            parse_canonical_ast(canonical_utf8.encode("utf-8"), registry)
    for direction in fixture["rejected_directions"]:
        with pytest.raises(ValueError):
            FactorDirection(direction)
