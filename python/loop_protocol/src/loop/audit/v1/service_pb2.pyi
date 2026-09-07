from loop.v1 import audit_pb2 as _audit_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class AppendAuditEventRequest(_message.Message):
    __slots__ = ("context", "expected_previous_sequence", "expected_previous_event_sha256", "action", "target", "payload", "audit_ledger_id")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_PREVIOUS_SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_PREVIOUS_EVENT_SHA256_FIELD_NUMBER: _ClassVar[int]
    ACTION_FIELD_NUMBER: _ClassVar[int]
    TARGET_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    AUDIT_LEDGER_ID_FIELD_NUMBER: _ClassVar[int]
    context: _common_pb2.CommandContext
    expected_previous_sequence: int
    expected_previous_event_sha256: _common_pb2.Sha256Digest
    action: _audit_pb2.AuditAction
    target: _audit_pb2.AuditTarget
    payload: _audit_pb2.AuditPayload
    audit_ledger_id: _common_pb2.AuditLedgerId
    def __init__(self, context: _Optional[_Union[_common_pb2.CommandContext, _Mapping]] = ..., expected_previous_sequence: _Optional[int] = ..., expected_previous_event_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., action: _Optional[_Union[_audit_pb2.AuditAction, str]] = ..., target: _Optional[_Union[_audit_pb2.AuditTarget, _Mapping]] = ..., payload: _Optional[_Union[_audit_pb2.AuditPayload, _Mapping]] = ..., audit_ledger_id: _Optional[_Union[_common_pb2.AuditLedgerId, _Mapping]] = ...) -> None: ...

class AppendAuditEventResponse(_message.Message):
    __slots__ = ("event",)
    EVENT_FIELD_NUMBER: _ClassVar[int]
    event: _audit_pb2.AuditEvent
    def __init__(self, event: _Optional[_Union[_audit_pb2.AuditEvent, _Mapping]] = ...) -> None: ...

class GetAuditEventRequest(_message.Message):
    __slots__ = ("audit_event_id",)
    AUDIT_EVENT_ID_FIELD_NUMBER: _ClassVar[int]
    audit_event_id: _common_pb2.AuditEventId
    def __init__(self, audit_event_id: _Optional[_Union[_common_pb2.AuditEventId, _Mapping]] = ...) -> None: ...

class GetAuditEventResponse(_message.Message):
    __slots__ = ("event",)
    EVENT_FIELD_NUMBER: _ClassVar[int]
    event: _audit_pb2.AuditEvent
    def __init__(self, event: _Optional[_Union[_audit_pb2.AuditEvent, _Mapping]] = ...) -> None: ...

class ListAuditEventsRequest(_message.Message):
    __slots__ = ("target", "page", "audit_ledger_id")
    TARGET_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    AUDIT_LEDGER_ID_FIELD_NUMBER: _ClassVar[int]
    target: _audit_pb2.AuditTarget
    page: _common_pb2.PageRequest
    audit_ledger_id: _common_pb2.AuditLedgerId
    def __init__(self, target: _Optional[_Union[_audit_pb2.AuditTarget, _Mapping]] = ..., page: _Optional[_Union[_common_pb2.PageRequest, _Mapping]] = ..., audit_ledger_id: _Optional[_Union[_common_pb2.AuditLedgerId, _Mapping]] = ...) -> None: ...

class ListAuditEventsResponse(_message.Message):
    __slots__ = ("events", "page")
    EVENTS_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    events: _containers.RepeatedCompositeFieldContainer[_audit_pb2.AuditEvent]
    page: _common_pb2.PageInfo
    def __init__(self, events: _Optional[_Iterable[_Union[_audit_pb2.AuditEvent, _Mapping]]] = ..., page: _Optional[_Union[_common_pb2.PageInfo, _Mapping]] = ...) -> None: ...
