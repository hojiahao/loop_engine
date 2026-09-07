import datetime

from google.protobuf import duration_pb2 as _duration_pb2
from google.protobuf import timestamp_pb2 as _timestamp_pb2
from loop.v1 import artifact_pb2 as _artifact_pb2
from loop.v1 import common_pb2 as _common_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ModelRole(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MODEL_ROLE_UNSPECIFIED: _ClassVar[ModelRole]
    MODEL_ROLE_SYSTEM: _ClassVar[ModelRole]
    MODEL_ROLE_USER: _ClassVar[ModelRole]
    MODEL_ROLE_ASSISTANT: _ClassVar[ModelRole]
    MODEL_ROLE_TOOL: _ClassVar[ModelRole]

class ImageDetail(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    IMAGE_DETAIL_UNSPECIFIED: _ClassVar[ImageDetail]
    IMAGE_DETAIL_AUTO: _ClassVar[ImageDetail]
    IMAGE_DETAIL_LOW: _ClassVar[ImageDetail]
    IMAGE_DETAIL_HIGH: _ClassVar[ImageDetail]
    IMAGE_DETAIL_ORIGINAL: _ClassVar[ImageDetail]

class ToolChoiceMode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    TOOL_CHOICE_MODE_UNSPECIFIED: _ClassVar[ToolChoiceMode]
    TOOL_CHOICE_MODE_AUTO: _ClassVar[ToolChoiceMode]
    TOOL_CHOICE_MODE_NONE: _ClassVar[ToolChoiceMode]
    TOOL_CHOICE_MODE_REQUIRED: _ClassVar[ToolChoiceMode]
    TOOL_CHOICE_MODE_NAMED: _ClassVar[ToolChoiceMode]

class ModelFinishReason(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MODEL_FINISH_REASON_UNSPECIFIED: _ClassVar[ModelFinishReason]
    MODEL_FINISH_REASON_STOP: _ClassVar[ModelFinishReason]
    MODEL_FINISH_REASON_LENGTH: _ClassVar[ModelFinishReason]
    MODEL_FINISH_REASON_TOOL_CALL: _ClassVar[ModelFinishReason]
    MODEL_FINISH_REASON_CONTENT_FILTER: _ClassVar[ModelFinishReason]

class ToolResultStatus(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    TOOL_RESULT_STATUS_UNSPECIFIED: _ClassVar[ToolResultStatus]
    TOOL_RESULT_STATUS_SUCCESS: _ClassVar[ToolResultStatus]
    TOOL_RESULT_STATUS_ERROR: _ClassVar[ToolResultStatus]

class ModelProtocolFamily(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    MODEL_PROTOCOL_FAMILY_UNSPECIFIED: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_OPENAI_RESPONSES: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_OPENAI_CHAT_COMPLETIONS: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_ANTHROPIC_MESSAGES: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_GOOGLE_GENERATE_CONTENT: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_GOOGLE_INTERACTIONS: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_AWS_BEDROCK_CONVERSE: _ClassVar[ModelProtocolFamily]
    MODEL_PROTOCOL_FAMILY_COHERE_V2_CHAT: _ClassVar[ModelProtocolFamily]
MODEL_ROLE_UNSPECIFIED: ModelRole
MODEL_ROLE_SYSTEM: ModelRole
MODEL_ROLE_USER: ModelRole
MODEL_ROLE_ASSISTANT: ModelRole
MODEL_ROLE_TOOL: ModelRole
IMAGE_DETAIL_UNSPECIFIED: ImageDetail
IMAGE_DETAIL_AUTO: ImageDetail
IMAGE_DETAIL_LOW: ImageDetail
IMAGE_DETAIL_HIGH: ImageDetail
IMAGE_DETAIL_ORIGINAL: ImageDetail
TOOL_CHOICE_MODE_UNSPECIFIED: ToolChoiceMode
TOOL_CHOICE_MODE_AUTO: ToolChoiceMode
TOOL_CHOICE_MODE_NONE: ToolChoiceMode
TOOL_CHOICE_MODE_REQUIRED: ToolChoiceMode
TOOL_CHOICE_MODE_NAMED: ToolChoiceMode
MODEL_FINISH_REASON_UNSPECIFIED: ModelFinishReason
MODEL_FINISH_REASON_STOP: ModelFinishReason
MODEL_FINISH_REASON_LENGTH: ModelFinishReason
MODEL_FINISH_REASON_TOOL_CALL: ModelFinishReason
MODEL_FINISH_REASON_CONTENT_FILTER: ModelFinishReason
TOOL_RESULT_STATUS_UNSPECIFIED: ToolResultStatus
TOOL_RESULT_STATUS_SUCCESS: ToolResultStatus
TOOL_RESULT_STATUS_ERROR: ToolResultStatus
MODEL_PROTOCOL_FAMILY_UNSPECIFIED: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_OPENAI_RESPONSES: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_OPENAI_CHAT_COMPLETIONS: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_ANTHROPIC_MESSAGES: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_GOOGLE_GENERATE_CONTENT: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_GOOGLE_INTERACTIONS: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_AWS_BEDROCK_CONVERSE: ModelProtocolFamily
MODEL_PROTOCOL_FAMILY_COHERE_V2_CHAT: ModelProtocolFamily

class JsonDocument(_message.Message):
    __slots__ = ("utf8_json", "canonical_sha256", "schema_id", "schema_sha256")
    UTF8_JSON_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_SHA256_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_SHA256_FIELD_NUMBER: _ClassVar[int]
    utf8_json: bytes
    canonical_sha256: _common_pb2.Sha256Digest
    schema_id: str
    schema_sha256: _common_pb2.Sha256Digest
    def __init__(self, utf8_json: _Optional[bytes] = ..., canonical_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., schema_id: _Optional[str] = ..., schema_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class JsonSchema(_message.Message):
    __slots__ = ("schema_id", "schema_version", "canonical_json", "schema_sha256")
    SCHEMA_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_JSON_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_SHA256_FIELD_NUMBER: _ClassVar[int]
    schema_id: str
    schema_version: int
    canonical_json: bytes
    schema_sha256: _common_pb2.Sha256Digest
    def __init__(self, schema_id: _Optional[str] = ..., schema_version: _Optional[int] = ..., canonical_json: _Optional[bytes] = ..., schema_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class TextContent(_message.Message):
    __slots__ = ("text",)
    TEXT_FIELD_NUMBER: _ClassVar[int]
    text: str
    def __init__(self, text: _Optional[str] = ...) -> None: ...

class ImageContent(_message.Message):
    __slots__ = ("artifact", "detail")
    ARTIFACT_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    artifact: _artifact_pb2.ArtifactRef
    detail: ImageDetail
    def __init__(self, artifact: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ..., detail: _Optional[_Union[ImageDetail, str]] = ...) -> None: ...

class DocumentContent(_message.Message):
    __slots__ = ("artifact",)
    ARTIFACT_FIELD_NUMBER: _ClassVar[int]
    artifact: _artifact_pb2.ArtifactRef
    def __init__(self, artifact: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ...) -> None: ...

class ReasoningContinuationReference(_message.Message):
    __slots__ = ("provider_continuation_id", "provider_id", "model_resolution_id", "state_sha256", "expires_at")
    PROVIDER_CONTINUATION_ID_FIELD_NUMBER: _ClassVar[int]
    PROVIDER_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_RESOLUTION_ID_FIELD_NUMBER: _ClassVar[int]
    STATE_SHA256_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    provider_continuation_id: _common_pb2.ProviderContinuationId
    provider_id: _common_pb2.ProviderId
    model_resolution_id: _common_pb2.ModelResolutionId
    state_sha256: _common_pb2.Sha256Digest
    expires_at: _timestamp_pb2.Timestamp
    def __init__(self, provider_continuation_id: _Optional[_Union[_common_pb2.ProviderContinuationId, _Mapping]] = ..., provider_id: _Optional[_Union[_common_pb2.ProviderId, _Mapping]] = ..., model_resolution_id: _Optional[_Union[_common_pb2.ModelResolutionId, _Mapping]] = ..., state_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., expires_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class ReasoningContent(_message.Message):
    __slots__ = ("text", "continuation")
    TEXT_FIELD_NUMBER: _ClassVar[int]
    CONTINUATION_FIELD_NUMBER: _ClassVar[int]
    text: str
    continuation: ReasoningContinuationReference
    def __init__(self, text: _Optional[str] = ..., continuation: _Optional[_Union[ReasoningContinuationReference, _Mapping]] = ...) -> None: ...

class ToolCallContent(_message.Message):
    __slots__ = ("tool_call_id", "tool_name", "arguments")
    TOOL_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_NAME_FIELD_NUMBER: _ClassVar[int]
    ARGUMENTS_FIELD_NUMBER: _ClassVar[int]
    tool_call_id: str
    tool_name: str
    arguments: JsonDocument
    def __init__(self, tool_call_id: _Optional[str] = ..., tool_name: _Optional[str] = ..., arguments: _Optional[_Union[JsonDocument, _Mapping]] = ...) -> None: ...

class ToolResultContent(_message.Message):
    __slots__ = ("tool_call_id", "status", "text", "json", "artifact")
    TOOL_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_FIELD_NUMBER: _ClassVar[int]
    TEXT_FIELD_NUMBER: _ClassVar[int]
    JSON_FIELD_NUMBER: _ClassVar[int]
    ARTIFACT_FIELD_NUMBER: _ClassVar[int]
    tool_call_id: str
    status: ToolResultStatus
    text: TextContent
    json: JsonDocument
    artifact: _artifact_pb2.ArtifactRef
    def __init__(self, tool_call_id: _Optional[str] = ..., status: _Optional[_Union[ToolResultStatus, str]] = ..., text: _Optional[_Union[TextContent, _Mapping]] = ..., json: _Optional[_Union[JsonDocument, _Mapping]] = ..., artifact: _Optional[_Union[_artifact_pb2.ArtifactRef, _Mapping]] = ...) -> None: ...

class RefusalContent(_message.Message):
    __slots__ = ("reason",)
    REASON_FIELD_NUMBER: _ClassVar[int]
    reason: str
    def __init__(self, reason: _Optional[str] = ...) -> None: ...

class StructuredOutputContent(_message.Message):
    __slots__ = ("output",)
    OUTPUT_FIELD_NUMBER: _ClassVar[int]
    output: JsonDocument
    def __init__(self, output: _Optional[_Union[JsonDocument, _Mapping]] = ...) -> None: ...

class ContentBlock(_message.Message):
    __slots__ = ("text", "image", "reasoning", "tool_call", "tool_result", "document", "refusal", "structured_output")
    TEXT_FIELD_NUMBER: _ClassVar[int]
    IMAGE_FIELD_NUMBER: _ClassVar[int]
    REASONING_FIELD_NUMBER: _ClassVar[int]
    TOOL_CALL_FIELD_NUMBER: _ClassVar[int]
    TOOL_RESULT_FIELD_NUMBER: _ClassVar[int]
    DOCUMENT_FIELD_NUMBER: _ClassVar[int]
    REFUSAL_FIELD_NUMBER: _ClassVar[int]
    STRUCTURED_OUTPUT_FIELD_NUMBER: _ClassVar[int]
    text: TextContent
    image: ImageContent
    reasoning: ReasoningContent
    tool_call: ToolCallContent
    tool_result: ToolResultContent
    document: DocumentContent
    refusal: RefusalContent
    structured_output: StructuredOutputContent
    def __init__(self, text: _Optional[_Union[TextContent, _Mapping]] = ..., image: _Optional[_Union[ImageContent, _Mapping]] = ..., reasoning: _Optional[_Union[ReasoningContent, _Mapping]] = ..., tool_call: _Optional[_Union[ToolCallContent, _Mapping]] = ..., tool_result: _Optional[_Union[ToolResultContent, _Mapping]] = ..., document: _Optional[_Union[DocumentContent, _Mapping]] = ..., refusal: _Optional[_Union[RefusalContent, _Mapping]] = ..., structured_output: _Optional[_Union[StructuredOutputContent, _Mapping]] = ...) -> None: ...

class ModelMessage(_message.Message):
    __slots__ = ("role", "content")
    ROLE_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    role: ModelRole
    content: _containers.RepeatedCompositeFieldContainer[ContentBlock]
    def __init__(self, role: _Optional[_Union[ModelRole, str]] = ..., content: _Optional[_Iterable[_Union[ContentBlock, _Mapping]]] = ...) -> None: ...

class ToolDefinition(_message.Message):
    __slots__ = ("name", "description", "input_schema", "strict")
    NAME_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    INPUT_SCHEMA_FIELD_NUMBER: _ClassVar[int]
    STRICT_FIELD_NUMBER: _ClassVar[int]
    name: str
    description: str
    input_schema: JsonSchema
    strict: bool
    def __init__(self, name: _Optional[str] = ..., description: _Optional[str] = ..., input_schema: _Optional[_Union[JsonSchema, _Mapping]] = ..., strict: _Optional[bool] = ...) -> None: ...

class ToolChoice(_message.Message):
    __slots__ = ("mode", "named_tool")
    MODE_FIELD_NUMBER: _ClassVar[int]
    NAMED_TOOL_FIELD_NUMBER: _ClassVar[int]
    mode: ToolChoiceMode
    named_tool: str
    def __init__(self, mode: _Optional[_Union[ToolChoiceMode, str]] = ..., named_tool: _Optional[str] = ...) -> None: ...

class StructuredOutputDefinition(_message.Message):
    __slots__ = ("name", "description", "json_schema", "strict")
    NAME_FIELD_NUMBER: _ClassVar[int]
    DESCRIPTION_FIELD_NUMBER: _ClassVar[int]
    JSON_SCHEMA_FIELD_NUMBER: _ClassVar[int]
    STRICT_FIELD_NUMBER: _ClassVar[int]
    name: str
    description: str
    json_schema: JsonSchema
    strict: bool
    def __init__(self, name: _Optional[str] = ..., description: _Optional[str] = ..., json_schema: _Optional[_Union[JsonSchema, _Mapping]] = ..., strict: _Optional[bool] = ...) -> None: ...

class ModelCapabilities(_message.Message):
    __slots__ = ("context_window_tokens", "maximum_output_tokens", "supports_tools", "supports_parallel_tools", "supports_structured_output", "supports_vision", "supports_reasoning", "supports_prompt_caching", "supports_streaming")
    CONTEXT_WINDOW_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_TOOLS_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_PARALLEL_TOOLS_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_STRUCTURED_OUTPUT_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_VISION_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_REASONING_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_PROMPT_CACHING_FIELD_NUMBER: _ClassVar[int]
    SUPPORTS_STREAMING_FIELD_NUMBER: _ClassVar[int]
    context_window_tokens: int
    maximum_output_tokens: int
    supports_tools: bool
    supports_parallel_tools: bool
    supports_structured_output: bool
    supports_vision: bool
    supports_reasoning: bool
    supports_prompt_caching: bool
    supports_streaming: bool
    def __init__(self, context_window_tokens: _Optional[int] = ..., maximum_output_tokens: _Optional[int] = ..., supports_tools: _Optional[bool] = ..., supports_parallel_tools: _Optional[bool] = ..., supports_structured_output: _Optional[bool] = ..., supports_vision: _Optional[bool] = ..., supports_reasoning: _Optional[bool] = ..., supports_prompt_caching: _Optional[bool] = ..., supports_streaming: _Optional[bool] = ...) -> None: ...

class ModelPricing(_message.Message):
    __slots__ = ("input_per_million_tokens", "output_per_million_tokens", "cached_input_per_million_tokens")
    INPUT_PER_MILLION_TOKENS_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_PER_MILLION_TOKENS_FIELD_NUMBER: _ClassVar[int]
    CACHED_INPUT_PER_MILLION_TOKENS_FIELD_NUMBER: _ClassVar[int]
    input_per_million_tokens: _common_pb2.Money
    output_per_million_tokens: _common_pb2.Money
    cached_input_per_million_tokens: _common_pb2.Money
    def __init__(self, input_per_million_tokens: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., output_per_million_tokens: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., cached_input_per_million_tokens: _Optional[_Union[_common_pb2.Money, _Mapping]] = ...) -> None: ...

class ModelResolutionSnapshot(_message.Message):
    __slots__ = ("resolution_id", "provider_id", "model_id", "requested_alias", "protocol_family", "capabilities", "pricing", "capability_sha256", "catalog_sha256", "resolved_at", "provider_plugin_name", "provider_plugin_version", "provider_plugin_sha256", "snapshot_sha256")
    RESOLUTION_ID_FIELD_NUMBER: _ClassVar[int]
    PROVIDER_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_ID_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_ALIAS_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_FAMILY_FIELD_NUMBER: _ClassVar[int]
    CAPABILITIES_FIELD_NUMBER: _ClassVar[int]
    PRICING_FIELD_NUMBER: _ClassVar[int]
    CAPABILITY_SHA256_FIELD_NUMBER: _ClassVar[int]
    CATALOG_SHA256_FIELD_NUMBER: _ClassVar[int]
    RESOLVED_AT_FIELD_NUMBER: _ClassVar[int]
    PROVIDER_PLUGIN_NAME_FIELD_NUMBER: _ClassVar[int]
    PROVIDER_PLUGIN_VERSION_FIELD_NUMBER: _ClassVar[int]
    PROVIDER_PLUGIN_SHA256_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_SHA256_FIELD_NUMBER: _ClassVar[int]
    resolution_id: _common_pb2.ModelResolutionId
    provider_id: _common_pb2.ProviderId
    model_id: _common_pb2.ModelId
    requested_alias: str
    protocol_family: ModelProtocolFamily
    capabilities: ModelCapabilities
    pricing: ModelPricing
    capability_sha256: _common_pb2.Sha256Digest
    catalog_sha256: _common_pb2.Sha256Digest
    resolved_at: _timestamp_pb2.Timestamp
    provider_plugin_name: str
    provider_plugin_version: str
    provider_plugin_sha256: _common_pb2.Sha256Digest
    snapshot_sha256: _common_pb2.Sha256Digest
    def __init__(self, resolution_id: _Optional[_Union[_common_pb2.ModelResolutionId, _Mapping]] = ..., provider_id: _Optional[_Union[_common_pb2.ProviderId, _Mapping]] = ..., model_id: _Optional[_Union[_common_pb2.ModelId, _Mapping]] = ..., requested_alias: _Optional[str] = ..., protocol_family: _Optional[_Union[ModelProtocolFamily, str]] = ..., capabilities: _Optional[_Union[ModelCapabilities, _Mapping]] = ..., pricing: _Optional[_Union[ModelPricing, _Mapping]] = ..., capability_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., catalog_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., resolved_at: _Optional[_Union[datetime.datetime, _timestamp_pb2.Timestamp, _Mapping]] = ..., provider_plugin_name: _Optional[str] = ..., provider_plugin_version: _Optional[str] = ..., provider_plugin_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ..., snapshot_sha256: _Optional[_Union[_common_pb2.Sha256Digest, _Mapping]] = ...) -> None: ...

class InvocationBudget(_message.Message):
    __slots__ = ("maximum_input_tokens", "maximum_output_tokens", "maximum_cost", "maximum_wall_time")
    MAXIMUM_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_COST_FIELD_NUMBER: _ClassVar[int]
    MAXIMUM_WALL_TIME_FIELD_NUMBER: _ClassVar[int]
    maximum_input_tokens: int
    maximum_output_tokens: int
    maximum_cost: _common_pb2.Money
    maximum_wall_time: _duration_pb2.Duration
    def __init__(self, maximum_input_tokens: _Optional[int] = ..., maximum_output_tokens: _Optional[int] = ..., maximum_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ..., maximum_wall_time: _Optional[_Union[datetime.timedelta, _duration_pb2.Duration, _Mapping]] = ...) -> None: ...

class ModelInvocation(_message.Message):
    __slots__ = ("request_id", "model", "messages", "tools", "tool_choice", "structured_output", "budget", "request_policy")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_FIELD_NUMBER: _ClassVar[int]
    MESSAGES_FIELD_NUMBER: _ClassVar[int]
    TOOLS_FIELD_NUMBER: _ClassVar[int]
    TOOL_CHOICE_FIELD_NUMBER: _ClassVar[int]
    STRUCTURED_OUTPUT_FIELD_NUMBER: _ClassVar[int]
    BUDGET_FIELD_NUMBER: _ClassVar[int]
    REQUEST_POLICY_FIELD_NUMBER: _ClassVar[int]
    request_id: _common_pb2.RequestId
    model: ModelResolutionSnapshot
    messages: _containers.RepeatedCompositeFieldContainer[ModelMessage]
    tools: _containers.RepeatedCompositeFieldContainer[ToolDefinition]
    tool_choice: ToolChoice
    structured_output: StructuredOutputDefinition
    budget: InvocationBudget
    request_policy: _common_pb2.PolicyReference
    def __init__(self, request_id: _Optional[_Union[_common_pb2.RequestId, _Mapping]] = ..., model: _Optional[_Union[ModelResolutionSnapshot, _Mapping]] = ..., messages: _Optional[_Iterable[_Union[ModelMessage, _Mapping]]] = ..., tools: _Optional[_Iterable[_Union[ToolDefinition, _Mapping]]] = ..., tool_choice: _Optional[_Union[ToolChoice, _Mapping]] = ..., structured_output: _Optional[_Union[StructuredOutputDefinition, _Mapping]] = ..., budget: _Optional[_Union[InvocationBudget, _Mapping]] = ..., request_policy: _Optional[_Union[_common_pb2.PolicyReference, _Mapping]] = ...) -> None: ...

class ModelUsage(_message.Message):
    __slots__ = ("input_tokens", "output_tokens", "cached_input_tokens", "reasoning_tokens", "charged_cost")
    INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    CACHED_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    REASONING_TOKENS_FIELD_NUMBER: _ClassVar[int]
    CHARGED_COST_FIELD_NUMBER: _ClassVar[int]
    input_tokens: int
    output_tokens: int
    cached_input_tokens: int
    reasoning_tokens: int
    charged_cost: _common_pb2.Money
    def __init__(self, input_tokens: _Optional[int] = ..., output_tokens: _Optional[int] = ..., cached_input_tokens: _Optional[int] = ..., reasoning_tokens: _Optional[int] = ..., charged_cost: _Optional[_Union[_common_pb2.Money, _Mapping]] = ...) -> None: ...

class ModelResponse(_message.Message):
    __slots__ = ("request_id", "resolution_id", "content", "finish_reason", "usage")
    REQUEST_ID_FIELD_NUMBER: _ClassVar[int]
    RESOLUTION_ID_FIELD_NUMBER: _ClassVar[int]
    CONTENT_FIELD_NUMBER: _ClassVar[int]
    FINISH_REASON_FIELD_NUMBER: _ClassVar[int]
    USAGE_FIELD_NUMBER: _ClassVar[int]
    request_id: _common_pb2.RequestId
    resolution_id: _common_pb2.ModelResolutionId
    content: _containers.RepeatedCompositeFieldContainer[ContentBlock]
    finish_reason: ModelFinishReason
    usage: ModelUsage
    def __init__(self, request_id: _Optional[_Union[_common_pb2.RequestId, _Mapping]] = ..., resolution_id: _Optional[_Union[_common_pb2.ModelResolutionId, _Mapping]] = ..., content: _Optional[_Iterable[_Union[ContentBlock, _Mapping]]] = ..., finish_reason: _Optional[_Union[ModelFinishReason, str]] = ..., usage: _Optional[_Union[ModelUsage, _Mapping]] = ...) -> None: ...
