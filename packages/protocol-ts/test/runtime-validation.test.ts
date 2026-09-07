import { readFileSync } from "node:fs";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";

import {
  ErrorCategory,
  ErrorDetailSchema,
  ServiceErrorSchema,
} from "../src/generated/loop/v1/common_pb.js";
import {
  ArtifactJobInputSchema,
  DiscoveryJobInputSchema,
  JobKind,
  JobSpecificationSchema,
} from "../src/generated/loop/v1/job_pb.js";
import {
  RuntimeValidationError,
  SERVICE_ERROR_TYPE_URL,
  validateJobWireDispatchCandidate,
  validateOperationalFailure,
} from "../src/runtime-validation.js";

const REPOSITORY_ROOT = new URL("../../../", import.meta.url);
const PROTOCOL_FIXTURES = new URL("fixtures/contracts/protocol/v1/", REPOSITORY_ROOT);

interface OperationalFixture {
  readonly grpc_status_code: number;
  readonly response_body_base64: string;
  readonly details: readonly {
    readonly type_url: string;
    readonly value_base64: string;
  }[];
  readonly expected: {
    readonly category: string;
    readonly code: string;
    readonly message: string;
    readonly retryable: boolean;
  };
}

function operationalFixture(): OperationalFixture {
  return JSON.parse(
    readFileSync(new URL("tests/contracts/operational_failure.json", REPOSITORY_ROOT), "ascii"),
  ) as OperationalFixture;
}

function expectValidationCode(operation: () => unknown, code: string): void {
  try {
    operation();
    throw new Error(`expected runtime validation error ${code}`);
  } catch (error) {
    expect(error).toBeInstanceOf(RuntimeValidationError);
    expect((error as RuntimeValidationError).code).toBe(code);
  }
}

describe("operational failure validation", () => {
  it("validates the shared ServiceError rich-status fixture", () => {
    const fixture = operationalFixture();
    const detail = fixture.details[0];
    if (detail === undefined) {
      throw new Error("operational fixture detail is required");
    }
    const validated = validateOperationalFailure(
      fixture.grpc_status_code,
      Buffer.from(fixture.response_body_base64, "base64"),
      [{ typeUrl: detail.type_url, value: Buffer.from(detail.value_base64, "base64") }],
    );

    expect(validated.category).toBe(ErrorCategory.DEPENDENCY);
    expect(validated.code).toBe(fixture.expected.code);
    expect(validated.message).toBe(fixture.expected.message);
    expect(validated.retryable).toBe(fixture.expected.retryable);
  });

  it("fails closed for forged types, OK status, and a non-empty body", () => {
    const fixture = operationalFixture();
    const detail = fixture.details[0];
    if (detail === undefined) {
      throw new Error("operational fixture detail is required");
    }
    const value = Buffer.from(detail.value_base64, "base64");
    for (const typeUrl of [
      "type.googleapis.com/loop.v1.FactorRejection",
      "type.googleapis.com/vendor.FutureError",
    ]) {
      expectValidationCode(
        () =>
          validateOperationalFailure(fixture.grpc_status_code, new Uint8Array(), [
            { typeUrl, value },
          ]),
        "unexpected_detail",
      );
    }
    expectValidationCode(
      () =>
        validateOperationalFailure(0, new Uint8Array(), [
          { typeUrl: SERVICE_ERROR_TYPE_URL, value },
        ]),
      "invalid_status",
    );
    expectValidationCode(
      () =>
        validateOperationalFailure(fixture.grpc_status_code, Uint8Array.of(1), [
          { typeUrl: SERVICE_ERROR_TYPE_URL, value },
        ]),
      "unexpected_response_body",
    );
  });

  it("rejects unbounded or malformed ServiceError fields", () => {
    const baseline = create(ServiceErrorSchema, {
      category: ErrorCategory.DEPENDENCY,
      code: "artifact_digest_mismatch",
      message: "artifact verification failed",
      retryable: true,
    });
    const valid = create(ServiceErrorSchema, {
      ...baseline,
      details: [
        create(ErrorDetailSchema, {
          fieldPath: "$.artifact.sha256",
          code: "digest_mismatch",
          message: "declared and computed digests differ",
        }),
      ],
    });
    expect(() => validateServiceErrorBytes(valid)).not.toThrow();

    expectValidationCode(
      () => validateServiceErrorBytes(create(ServiceErrorSchema, { ...baseline, category: 999 })),
      "unsupported_enum",
    );

    const invalid = [
      create(ServiceErrorSchema, { ...baseline, code: "Invalid Code" }),
      create(ServiceErrorSchema, { ...baseline, code: "a".repeat(129) }),
      create(ServiceErrorSchema, { ...baseline, message: "é".repeat(1_025) }),
      create(ServiceErrorSchema, { ...baseline, message: "forged\nrecord" }),
      create(ServiceErrorSchema, { ...baseline, details: [create(ErrorDetailSchema)] }),
      create(ServiceErrorSchema, {
        ...baseline,
        details: [
          create(ErrorDetailSchema, {
            fieldPath: "$.field\tname",
            code: "invalid_field",
            message: "invalid field",
          }),
        ],
      }),
      create(ServiceErrorSchema, {
        ...baseline,
        details: [
          create(ErrorDetailSchema, {
            fieldPath: "x".repeat(513),
            code: "invalid_field",
            message: "invalid field",
          }),
        ],
      }),
      create(ServiceErrorSchema, {
        ...baseline,
        details: [
          create(ErrorDetailSchema, {
            fieldPath: "$.field",
            code: "INVALID",
            message: "invalid code",
          }),
        ],
      }),
      create(ServiceErrorSchema, {
        ...baseline,
        details: [
          create(ErrorDetailSchema, {
            fieldPath: "$.field",
            code: "invalid_field",
            message: " ",
          }),
        ],
      }),
    ];
    for (const serviceError of invalid) {
      expectValidationCode(() => validateServiceErrorBytes(serviceError), "invalid_service_error");
    }
  });
});

function validateServiceErrorBytes(serviceError: Parameters<typeof toBinary>[1]): void {
  validateOperationalFailure(14, new Uint8Array(), [
    {
      typeUrl: SERVICE_ERROR_TYPE_URL,
      value: toBinary(ServiceErrorSchema, serviceError),
    },
  ]);
}

describe("job dispatch validation", () => {
  it("rejects the shared unknown enum and unknown oneof fixtures", () => {
    const unknownEnum = fromBinary(
      JobSpecificationSchema,
      readFileSync(new URL("job_specification_v1_unknown_enum.binpb", PROTOCOL_FIXTURES)),
    );
    expect(unknownEnum.kind).toBe(127);
    expect(unknownEnum.input.case).toBe("discovery");
    expectValidationCode(
      () => validateJobWireDispatchCandidate(unknownEnum, new Set([JobKind.DISCOVERY])),
      "unsupported_enum",
    );

    const unknownOneof = fromBinary(
      JobSpecificationSchema,
      readFileSync(new URL("job_specification_v1_unknown_oneof.binpb", PROTOCOL_FIXTURES)),
    );
    expect(unknownOneof.kind).toBe(JobKind.REPORT);
    expect(unknownOneof.input.case).toBeUndefined();
    expectValidationCode(
      () => validateJobWireDispatchCandidate(unknownOneof, new Set([JobKind.REPORT])),
      "unsupported_oneof",
    );
  });

  it("requires a complete envelope after matching known discriminants", () => {
    const valid = create(JobSpecificationSchema, {
      kind: JobKind.DISCOVERY,
      input: { case: "discovery", value: create(DiscoveryJobInputSchema) },
    });
    expectValidationCode(
      () => validateJobWireDispatchCandidate(valid, new Set([JobKind.DISCOVERY])),
      "invalid_specification",
    );

    const mismatched = create(JobSpecificationSchema, {
      kind: JobKind.DISCOVERY,
      input: { case: "artifact", value: create(ArtifactJobInputSchema) },
    });
    expectValidationCode(
      () => validateJobWireDispatchCandidate(mismatched, new Set([JobKind.DISCOVERY])),
      "incompatible_variant",
    );

    const disabled = create(JobSpecificationSchema, {
      kind: JobKind.PROSPECTIVE_OBSERVATION,
      input: { case: "artifact", value: create(ArtifactJobInputSchema) },
    });
    expectValidationCode(
      () => validateJobWireDispatchCandidate(disabled, new Set([JobKind.DISCOVERY])),
      "invalid_specification",
    );
  });
});
