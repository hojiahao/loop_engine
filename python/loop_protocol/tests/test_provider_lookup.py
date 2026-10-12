"""Consume the shared recovery wire fixtures without granting retry authority."""

from pathlib import Path

from loop.provider.v1 import service_pb2

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/contracts/protocol/v1"


def test_lookup_request() -> None:
    request = service_pb2.LookupInvocationRequest.FromString(
        (FIXTURES / "provider_lookup_v1.binpb").read_bytes()
    )
    assert request.context.request_id.value == "lookup.1"
    assert request.original_request_id.value == "invoke.1"
    assert request.original_idempotency_key.value == "invoke-key.1"
    assert request.request_sha256.value == bytes(range(32))
    assert service_pb2.LookupInvocationRequest.FromString(request.SerializeToString()) == request
    method = service_pb2.DESCRIPTOR.services_by_name["ProviderService"].methods_by_name[
        "LookupInvocation"
    ]
    assert method.input_type is service_pb2.LookupInvocationRequest.DESCRIPTOR
    assert not method.client_streaming
    assert not method.server_streaming


def test_lookup_result() -> None:
    result = service_pb2.LookupInvocationResponse.FromString(
        (FIXTURES / "provider_completed_v1.binpb").read_bytes()
    )
    assert result.state == service_pb2.INVOCATION_STATE_COMPLETED
    assert result.response.request_id.value == "invoke.1"
    assert result.response.content[0].text.text == "fixture result"
    assert not result.response.usage.HasField("charged_cost")
    assert result.reserved_cost.amount.value == "0.125"
    assert result.reserved_cost.currency_code == "USD"
    assert service_pb2.LookupInvocationResponse.FromString(result.SerializeToString()) == result


def test_lookup_unknown() -> None:
    result = service_pb2.LookupInvocationResponse.FromString(bytes((8, 127)))
    assert result.state == 127
    assert result.state not in service_pb2.InvocationState.values()
    assert not result.HasField("response")
    assert not result.HasField("reserved_cost")
