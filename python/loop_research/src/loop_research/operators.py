"""Executable US operator semantics; no caller-supplied implementation registry."""

from functools import lru_cache

from loop_protocol.canonical import (
    AlignmentPolicy,
    ArgumentDefinition,
    DecimalConstraints,
    FieldDefinition,
    NullPolicy,
    NumericPolicy,
    OperatorDefinition,
    OperatorRegistry,
    OperatorSemanticContract,
    ScalarValueType,
    TiePolicy,
    WindowPolicy,
)

VERSION = "2"
FIELDS = (
    "market.open",
    "market.high",
    "market.low",
    "market.close",
    "market.adjusted_close",
    "market.volume",
)
ROLLING = ("ma", "std", "min", "max", "skew", "rank_ts")
LAGGED = ("roc", "delta")
BINARY = ("add", "sub", "mul", "div")
CROSS_SECTIONAL = ("zscore", "rank_cs")


@lru_cache(maxsize=1)
def semantic_contracts() -> tuple[OperatorSemanticContract, ...]:
    """Return immutable contracts implemented by this installed worker version."""
    result = []
    for name in sorted((*ROLLING, *LAGGED, *BINARY, *CROSS_SECTIONAL)):
        null = NullPolicy.PROPAGATE
        window = WindowPolicy.NOT_APPLICABLE
        tie = TiePolicy.NOT_APPLICABLE
        alignment = AlignmentPolicy.UNARY_PRESERVE_TIMESTAMP_AND_SECURITY
        numeric = NumericPolicy.BINARY64_NON_FINITE_TO_MISSING
        if name in ROLLING:
            null = NullPolicy.IGNORE_MISSING
            window = WindowPolicy.TRAILING_EXPLICIT_MINIMUM
        elif name in LAGGED:
            window = WindowPolicy.LAG_ARGUMENT_2
        elif name in BINARY:
            alignment = AlignmentPolicy.STRICT_TIMESTAMP_AND_SECURITY
        if name in ("rank_ts", "rank_cs", "zscore"):
            null = NullPolicy.PRESERVE_TARGET_IGNORE_PEERS
        if name == "std":
            numeric = NumericPolicy.SAMPLE_STD
        elif name == "skew":
            numeric = NumericPolicy.ADJUSTED_SKEW
        elif name == "zscore":
            numeric = NumericPolicy.SAMPLE_ZSCORE
        elif name == "rank_ts":
            tie = TiePolicy.TARGET_LAST_STABLE_ORDER_VALID_COUNT_MINUS_ONE_CONSTANT_MIDPOINT
            numeric = NumericPolicy.ORDINAL_UNIT_INTERVAL
        elif name == "rank_cs":
            tie = TiePolicy.AVERAGE_VALID_COUNT
            numeric = NumericPolicy.ORDINAL_UNIT_INTERVAL
        result.append(
            OperatorSemanticContract(name, VERSION, null, window, tie, alignment, numeric)
        )
    return tuple(result)


@lru_cache(maxsize=1)
def operator_registry() -> OperatorRegistry:
    """Resolve only implemented semantic versions, with explicit typed windows.

    Arithmetic is binary and not associative in binary64. Addition and
    multiplication may commute their two operands, but never flatten nested
    operations. Unknown fields/versions remain unavailable until implemented.
    """
    series = ArgumentDefinition(ScalarValueType.SERIES)
    count = ArgumentDefinition(
        ScalarValueType.DECIMAL,
        literal_only=True,
        decimal=DecimalConstraints(4, 0, "1", "4096"),
    )
    operators = []
    contracts = semantic_contracts()
    for contract in contracts:
        name = contract.operator
        parameters: tuple[ArgumentDefinition, ...]
        if name in ROLLING:
            parameters = (series, count, count)
        elif name in LAGGED:
            parameters = (series, count)
        elif name in BINARY:
            parameters = (series, series)
        else:
            parameters = (series,)
        operators.append(
            OperatorDefinition(
                name,
                VERSION,
                parameters,
                ScalarValueType.SERIES,
                contract.sha256,
                commutative=name in ("add", "mul"),
            )
        )
    documents = {contract.sha256: contract.canonical_bytes for contract in contracts}
    return OperatorRegistry(
        fields=tuple(FieldDefinition(name, ScalarValueType.SERIES) for name in FIELDS),
        enums=(),
        operators=tuple(operators),
        semantic_contract_resolver=documents.get,
    )
