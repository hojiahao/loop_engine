import hashlib
from dataclasses import replace
from types import SimpleNamespace
from typing import cast

import pytest

from loop_protocol import (
    AlignmentPolicy,
    ArgumentDefinition,
    BooleanNode,
    CallNode,
    CanonicalFactorSpec,
    CanonicalizationError,
    CanonicalizationLimits,
    DecimalConstraints,
    DecimalNode,
    EnumDefinition,
    EnumNode,
    EnumValueType,
    FactorDirection,
    FactorSpec,
    FieldDefinition,
    FieldNode,
    NullPolicy,
    NumericPolicy,
    OperatorPolicy,
    OperatorRegistry,
    OperatorSemanticContract,
    PolicyReference,
    ScalarValueType,
    TiePolicy,
    WindowPolicy,
    bind_factor_spec,
    canonicalize_expression,
    parse_canonical_ast,
    parse_canonical_factor_spec,
    resolve_canonicalization_limits,
    verify_expression_id,
    verify_factor_spec_id,
)


@pytest.fixture
def registry() -> OperatorRegistry:
    series = ArgumentDefinition(ScalarValueType.SERIES)
    window = ArgumentDefinition(
        ScalarValueType.DECIMAL,
        literal_only=True,
        decimal=DecimalConstraints(3, 0, "2", "252"),
    )
    contracts: dict[str, bytes] = {}

    def semantic(operator: str, version: str) -> str:
        if operator == "rolling.mean":
            contract = OperatorSemanticContract(
                operator,
                version,
                NullPolicy.IGNORE_MISSING,
                WindowPolicy.TRAILING_ARGUMENT_2_FULL_WINDOW_RIGHT_INCLUSIVE_CONSTANT_PRESERVE,
                TiePolicy.NOT_APPLICABLE,
                AlignmentPolicy.UNARY_PRESERVE_TIMESTAMP_AND_SECURITY,
                NumericPolicy.BINARY64_NON_FINITE_TO_MISSING,
            )
        elif operator == "rank.cross_section":
            contract = OperatorSemanticContract(
                operator,
                version,
                NullPolicy.PRESERVE_TARGET_IGNORE_PEERS,
                WindowPolicy.NOT_APPLICABLE,
                TiePolicy.ARGUMENT_2_AVERAGE_OR_DENSE_VALID_COUNT_CONSTANT_MIDPOINT,
                AlignmentPolicy.UNARY_PRESERVE_TIMESTAMP_AND_SECURITY,
                NumericPolicy.ORDINAL_UNIT_INTERVAL,
            )
        else:
            contract = OperatorSemanticContract(
                operator,
                version,
                NullPolicy.PROPAGATE,
                WindowPolicy.NOT_APPLICABLE,
                TiePolicy.NOT_APPLICABLE,
                AlignmentPolicy.STRICT_TIMESTAMP_AND_SECURITY,
                NumericPolicy.BINARY64_NON_FINITE_TO_MISSING,
            )
        contracts[contract.sha256] = contract.canonical_bytes
        return contract.sha256

    return OperatorRegistry(
        fields=(
            FieldDefinition("market.close", ScalarValueType.SERIES),
            FieldDefinition("market.volume", ScalarValueType.SERIES),
        ),
        enums=(EnumDefinition("rank.method", ("average", "dense")),),
        operators=(
            OperatorPolicy(
                "rolling.mean",
                "1",
                (series, window),
                ScalarValueType.SERIES,
                semantic("rolling.mean", "1"),
            ),
            OperatorPolicy(
                "arithmetic.add",
                "1",
                (),
                ScalarValueType.SERIES,
                semantic("arithmetic.add", "1"),
                variadic=series,
                minimum_arguments=2,
                maximum_arguments=8,
                associative=True,
                commutative=True,
            ),
            OperatorPolicy(
                "arithmetic.subtract",
                "1",
                (series, series),
                ScalarValueType.SERIES,
                semantic("arithmetic.subtract", "1"),
            ),
            OperatorPolicy(
                "rank.cross_section",
                "2",
                (
                    series,
                    ArgumentDefinition(
                        EnumValueType("rank.method"),
                        literal_only=True,
                    ),
                ),
                ScalarValueType.SERIES,
                semantic("rank.cross_section", "2"),
            ),
        ),
        semantic_contract_resolver=contracts.get,
    )


def test_documented_call_has_exact_canonical_bytes(registry: OperatorRegistry) -> None:
    expression = canonicalize_expression(
        CallNode("rolling.mean", "1", (FieldNode("market.close"), DecimalNode("20"))),
        registry,
    )

    assert expression.canonical_bytes == (
        b'{"node":"call","operator":"rolling.mean","operator_version":"1",'
        b'"arguments":[{"node":"field","field":"market.close"},'
        b'{"node":"decimal","value":"20"}]}'
    )
    assert expression.expression_id.startswith("sha256:")
    verified = verify_expression_id(
        expression.expression_id,
        expression.canonical_bytes,
        registry,
    )
    assert verified == expression
    assert parse_canonical_ast(expression.canonical_bytes, registry) == expression.ast


def test_commutative_associative_policy_converges(registry: OperatorRegistry) -> None:
    close = FieldNode("market.close")
    volume = FieldNode("market.volume")
    one = FieldNode("market.close")
    left_nested = CallNode(
        "arithmetic.add",
        "1",
        (CallNode("arithmetic.add", "1", (close, volume)), one),
    )
    permuted = CallNode(
        "arithmetic.add",
        "1",
        (volume, CallNode("arithmetic.add", "1", (one, close))),
    )

    left = canonicalize_expression(left_nested, registry)
    right = canonicalize_expression(permuted, registry)

    assert left.expression_id == right.expression_id
    assert left.canonical_bytes == right.canonical_bytes
    assert isinstance(left.ast, CallNode)
    assert len(left.ast.arguments) == 3


def test_unregistered_rewrite_preserves_order(registry: OperatorRegistry) -> None:
    first = canonicalize_expression(
        CallNode(
            "arithmetic.subtract",
            "1",
            (FieldNode("market.close"), FieldNode("market.volume")),
        ),
        registry,
    )
    second = canonicalize_expression(
        CallNode(
            "arithmetic.subtract",
            "1",
            (FieldNode("market.volume"), FieldNode("market.close")),
        ),
        registry,
    )

    assert first.expression_id != second.expression_id


@pytest.mark.parametrize(
    "value",
    ["", "01", "+1", "1.", ".5", "1.0", "-0", "1e2", "NaN", "Infinity"],
)
def test_noncanonical_decimal_is_rejected(registry: OperatorRegistry, value: str) -> None:
    with pytest.raises(CanonicalizationError):
        canonicalize_expression(DecimalNode(value), registry)


@pytest.mark.parametrize(
    "node",
    [
        FieldNode("Market.close"),
        FieldNode("market..close"),
        FieldNode("market/close"),
        FieldNode("market.clos\N{LATIN SMALL LETTER E WITH ACUTE}"),
        EnumNode("rank.method", "unknown"),
        CallNode("unknown.operator", "1", (BooleanNode(True),)),
        CallNode("rolling.mean", "01", (FieldNode("market.close"), DecimalNode("20"))),
    ],
)
def test_unknown_or_noncanonical_semantics_are_rejected(
    registry: OperatorRegistry, node: object
) -> None:
    with pytest.raises(CanonicalizationError):
        canonicalize_expression(node, registry)  # type: ignore[arg-type]


def test_canonical_parser_rejects_numbers_unknown_keys_and_whitespace(
    registry: OperatorRegistry,
) -> None:
    invalid = (
        b'{"node":"decimal","value":20}',
        b'{"field":"market.close","node":"field"}',
        b'{"node":"field","field":"market.close","extra":"x"}',
        b' {"node":"field","field":"market.close"}',
        b'{"node":"field","field":"market.close","field":"market.volume"}',
    )
    for canonical_bytes in invalid:
        with pytest.raises(CanonicalizationError):
            parse_canonical_ast(canonical_bytes, registry)


def test_depth_and_direct_argument_limits_fail_closed(registry: OperatorRegistry) -> None:
    too_deep: object = FieldNode("market.close")
    for _ in range(64):
        too_deep = CallNode(
            "rank.cross_section",
            "2",
            (too_deep, EnumNode("rank.method", "average")),  # type: ignore[arg-type]
        )
    with pytest.raises(CanonicalizationError, match="depth"):
        canonicalize_expression(too_deep, registry)  # type: ignore[arg-type]

    too_many = CallNode(
        "arithmetic.add",
        "1",
        tuple(DecimalNode("1") for _ in range(1_025)),
    )
    with pytest.raises(CanonicalizationError, match="direct arguments"):
        canonicalize_expression(too_many, registry)


def test_deployment_limits_are_bounded_by_v1_hard_maxima() -> None:
    with pytest.raises(CanonicalizationError, match="max_nodes"):
        CanonicalizationLimits(max_nodes=0)
    with pytest.raises(CanonicalizationError, match="max_depth"):
        CanonicalizationLimits(max_depth=65)
    with pytest.raises(CanonicalizationError, match="max_canonical_bytes"):
        CanonicalizationLimits(max_canonical_bytes=256 * 1_024 + 1)
    with pytest.raises(CanonicalizationError, match="max_direct_arguments"):
        CanonicalizationLimits(max_direct_arguments=1_025)
    with pytest.raises(CanonicalizationError, match="max_nodes"):
        CanonicalizationLimits(max_nodes=True)
    with pytest.raises(CanonicalizationError, match="unknown canonicalization limit"):
        resolve_canonicalization_limits({"unknown": 1})


def test_lower_deployment_limits_apply_during_normalization(
    registry: OperatorRegistry,
) -> None:
    expression = CallNode(
        "arithmetic.subtract",
        "1",
        (FieldNode("market.close"), FieldNode("market.volume")),
    )

    with pytest.raises(CanonicalizationError, match="depth"):
        canonicalize_expression(expression, registry, {"max_depth": 1})
    with pytest.raises(CanonicalizationError, match="node count"):
        canonicalize_expression(expression, registry, CanonicalizationLimits(max_nodes=2))
    with pytest.raises(CanonicalizationError, match="direct arguments"):
        canonicalize_expression(expression, registry, {"max_direct_arguments": 1})
    with pytest.raises(CanonicalizationError, match="canonical AST exceeds"):
        canonicalize_expression(expression, registry, {"max_canonical_bytes": 1})


def test_factor_spec_binds_direction_and_all_policy_digests(
    registry: OperatorRegistry,
) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    references = tuple(
        PolicyReference(f"policy_{index}", "1", f"sha256:{index:064x}") for index in range(1, 10)
    )
    spec = FactorSpec(
        expression.expression_id,
        registry.sha256,
        FactorDirection.HIGHER_IS_BETTER,
        *references,
    )

    canonical = bind_factor_spec(spec, expression.canonical_bytes, registry)
    verify_factor_spec_id(
        canonical.factor_spec_id,
        canonical.canonical_bytes,
        expression.canonical_bytes,
        registry,
    )
    assert b'"direction":"higher_is_better"' in canonical.canonical_bytes

    opposite = bind_factor_spec(
        replace(spec, direction=FactorDirection.LOWER_IS_BETTER),
        expression.canonical_bytes,
        registry,
    )
    changed_policy = bind_factor_spec(
        replace(
            spec,
            cost_policy=replace(spec.cost_policy, sha256=f"sha256:{99:064x}"),
        ),
        expression.canonical_bytes,
        registry,
    )
    assert opposite.factor_spec_id != canonical.factor_spec_id
    assert changed_policy.factor_spec_id != canonical.factor_spec_id


def test_identity_verification_rejects_mismatch(registry: OperatorRegistry) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    with pytest.raises(CanonicalizationError, match="does not match"):
        verify_expression_id(
            "sha256:" + "0" * 64,
            expression.canonical_bytes,
            registry,
        )


def test_expression_identity_verifier_rejects_hash_matching_invalid_bytes(
    registry: OperatorRegistry,
) -> None:
    invalid_bytes = b"not-json"
    claimed = "sha256:" + hashlib.sha256(b"loop.factor-ast/v1\x00" + invalid_bytes).hexdigest()

    with pytest.raises(CanonicalizationError, match="invalid canonical AST JSON"):
        verify_expression_id(claimed, invalid_bytes, registry)


@pytest.mark.parametrize(
    "policy_index,policy_name",
    tuple(
        enumerate(
            (
                "universe_policy",
                "data_policy",
                "calendar_policy",
                "preprocess_policy",
                "neutralization_policy",
                "portfolio_policy",
                "execution_policy",
                "cost_policy",
                "evaluation_policy",
            )
        )
    ),
)
def test_factor_spec_and_binding_validate_every_policy_reference(
    registry: OperatorRegistry,
    policy_index: int,
    policy_name: str,
) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    references = [
        PolicyReference(f"policy_{index}", "1", f"sha256:{index:064x}") for index in range(1, 10)
    ]
    invalid_reference = cast(
        PolicyReference,
        SimpleNamespace(policy_id="INVALID", revision="0", sha256="not-a-digest"),
    )
    invalid_references = references.copy()
    invalid_references[policy_index] = invalid_reference

    with pytest.raises(CanonicalizationError, match=policy_name):
        FactorSpec(
            expression.expression_id,
            registry.sha256,
            FactorDirection.HIGHER_IS_BETTER,
            *invalid_references,
        )

    valid_spec = FactorSpec(
        expression.expression_id,
        registry.sha256,
        FactorDirection.HIGHER_IS_BETTER,
        *references,
    )
    object.__setattr__(valid_spec, policy_name, invalid_reference)
    with pytest.raises(CanonicalizationError, match=policy_name):
        bind_factor_spec(valid_spec, expression.canonical_bytes, registry)


def test_lower_limits_reach_parse_bind_and_verify_entrypoints(
    registry: OperatorRegistry,
) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    references = tuple(
        PolicyReference(f"policy_{index}", "1", f"sha256:{index:064x}") for index in range(1, 10)
    )
    spec = FactorSpec(
        expression.expression_id,
        registry.sha256,
        FactorDirection.HIGHER_IS_BETTER,
        *references,
    )
    bound = bind_factor_spec(spec, expression.canonical_bytes, registry)
    expression_limit = {"max_canonical_bytes": len(expression.canonical_bytes) - 1}

    with pytest.raises(CanonicalizationError, match="canonical AST exceeds"):
        parse_canonical_ast(expression.canonical_bytes, registry, expression_limit)
    with pytest.raises(CanonicalizationError, match="canonical AST exceeds"):
        verify_expression_id(
            expression.expression_id,
            expression.canonical_bytes,
            registry,
            expression_limit,
        )
    with pytest.raises(CanonicalizationError, match="canonical AST exceeds"):
        bind_factor_spec(spec, expression.canonical_bytes, registry, expression_limit)

    spec_limit = {"max_canonical_bytes": len(bound.canonical_bytes) - 1}
    with pytest.raises(CanonicalizationError, match="canonical FactorSpec exceeds"):
        bind_factor_spec(spec, expression.canonical_bytes, registry, spec_limit)
    with pytest.raises(CanonicalizationError, match="canonical FactorSpec exceeds"):
        parse_canonical_factor_spec(
            bound.canonical_bytes,
            bound.factor_spec_id,
            expression.canonical_bytes,
            registry,
            spec_limit,
        )
    with pytest.raises(CanonicalizationError, match="canonical FactorSpec exceeds"):
        verify_factor_spec_id(
            bound.factor_spec_id,
            bound.canonical_bytes,
            expression.canonical_bytes,
            registry,
            spec_limit,
        )


def test_factor_binding_rejects_expression_identity_mismatch(registry: OperatorRegistry) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    references = tuple(
        PolicyReference(f"policy_{index}", "1", f"sha256:{index:064x}") for index in range(1, 10)
    )
    draft = FactorSpec(
        "sha256:" + "0" * 64,
        registry.sha256,
        FactorDirection.HIGHER_IS_BETTER,
        *references,
    )
    with pytest.raises(CanonicalizationError, match="expression_id does not match"):
        bind_factor_spec(draft, expression.canonical_bytes, registry)


def test_bound_factor_spec_has_no_public_constructor() -> None:
    with pytest.raises(TypeError):
        CanonicalFactorSpec()  # type: ignore[call-arg]


def test_canonical_factor_spec_parser_rejects_whitespace(registry: OperatorRegistry) -> None:
    expression = canonicalize_expression(FieldNode("market.close"), registry)
    references = tuple(
        PolicyReference(f"policy_{index}", "1", f"sha256:{index:064x}") for index in range(1, 10)
    )
    draft = FactorSpec(
        expression.expression_id,
        registry.sha256,
        FactorDirection.HIGHER_IS_BETTER,
        *references,
    )
    bound = bind_factor_spec(draft, expression.canonical_bytes, registry)
    with pytest.raises(CanonicalizationError):
        parse_canonical_factor_spec(
            b" " + bound.canonical_bytes,
            bound.factor_spec_id,
            expression.canonical_bytes,
            registry,
        )


def test_hostile_json_nesting_fails_closed_without_recursion_escape(
    registry: OperatorRegistry,
) -> None:
    hostile = b'{"node":"call","operator":"math.abs","operator_version":"1","arguments":[' * 2_000
    hostile += b'{"node":"field","field":"market.close"}' + b"]}" * 2_000
    with pytest.raises(CanonicalizationError):
        parse_canonical_ast(hostile, registry)
