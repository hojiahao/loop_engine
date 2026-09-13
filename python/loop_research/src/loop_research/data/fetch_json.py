"""Strict vendor JSON decoding without floating-point loss or unbounded decimals."""

import json
from decimal import Decimal
from typing import Any


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate vendor JSON field")
        result[key] = value
    return result


def _decimal(value: str) -> Decimal:
    if len(value) > 64:
        raise ValueError("vendor number exceeds the precision budget")
    number = Decimal(value)
    exponent = number.as_tuple().exponent
    if not number.is_finite() or not isinstance(exponent, int) or not -18 <= exponent <= 24:
        raise ValueError("vendor number has unsupported precision")
    if number and not -18 <= number.adjusted() <= 24:
        raise ValueError("vendor number exceeds the magnitude budget")
    return number


def _integer(value: str) -> int:
    if len(value.lstrip("-")) > 25:
        raise ValueError("vendor integer exceeds the precision budget")
    return int(value)


def _constant(value: str) -> None:
    raise ValueError("non-finite vendor JSON number")


def decode_object(content: bytes) -> dict[str, Any]:
    """Decode one already byte-bounded response; unknown metadata stays untrusted.

    Vendor schemas are parsed by each adapter after this syntax/number gate.
    Raw bytes remain the provenance source; this object is never reserialized
    and mislabeled as the original HTTP response.
    """
    try:
        value = json.loads(
            content.decode("utf-8"),
            object_pairs_hook=_object,
            parse_float=_decimal,
            parse_int=_integer,
            parse_constant=_constant,
        )
    except (UnicodeError, RecursionError) as error:
        raise ValueError("invalid vendor JSON encoding or depth") from error
    if not isinstance(value, dict):
        raise ValueError("vendor response requires a JSON object")
    return value


def decimal_text(value: object) -> str:
    """Render exact, bounded upstream numbers into the domain decimal format."""
    if type(value) is int or isinstance(value, Decimal):
        number = _decimal(str(value))
    else:
        raise ValueError("observation requires an exact JSON number")
    result = format(number, "f")
    if len(result) > 64:
        raise ValueError("observation decimal exceeds the byte budget")
    return result


def contains_secret(content: bytes, secrets: tuple[str, ...]) -> bool:
    """Check exact raw and JSON-decoded credential echoes before cache publication.

    The byte-bounded JSON decoder rejects ambiguity before the iterative scan;
    escaped strings cannot bypass this check. This is not a general classifier
    for arbitrary encoded or unrelated confidential vendor data.
    """
    if any(secret.encode() in content for secret in secrets):
        return True
    pending: list[object] = [decode_object(content)]
    while pending:
        value = pending.pop()
        if isinstance(value, str) and any(secret in value for secret in secrets):
            return True
        if isinstance(value, dict):
            pending.extend(value.keys())
            pending.extend(value.values())
        elif isinstance(value, list):
            pending.extend(value)
    return False
