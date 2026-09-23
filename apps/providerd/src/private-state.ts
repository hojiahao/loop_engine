import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { join } from "node:path";
import { create } from "@bufbuild/protobuf";
import { timestampDate, timestampFromDate } from "@bufbuild/protobuf/wkt";
import { Code } from "@connectrpc/connect";
import {
  type ArtifactRef,
  ErrorCategory,
  type ModelResolutionSnapshot,
  type ReasoningContent,
  ReasoningContinuationReferenceSchema,
} from "@loop-engine/protocol/provider";

import { type Deployment, read_private } from "./config.js";
import { ProviderError } from "./errors.js";
import { hex_digest } from "./identity.js";
import { finish_invocation } from "./journal.js";
import { type JsonValue, json_bytes, json_digest, parse_json } from "./json.js";

const prompt_media = [
  "text/plain",
  "application/json",
  "image/png",
  "image/jpeg",
  "application/pdf",
] as const;

export function prompt_schema(media: string): Uint8Array {
  return json_digest(json_bytes({ schema: "loop.prompt-artifact/v1", media_type: media }));
}

export function actor_directory(actor: string): string {
  return createHash("sha256").update(actor).digest("hex");
}

function access_denied(): never {
  throw new ProviderError(
    "provider_private_state_denied",
    Code.PermissionDenied,
    ErrorCategory.AUTHORIZATION,
  );
}

async function private_directory(path: string): Promise<void> {
  const handle = await open(
    path,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    const info = await handle.stat();
    if ((info.mode & 0o077) !== 0 || info.uid !== process.getuid?.()) access_denied();
  } finally {
    await handle.close();
  }
}

/** References authorize nothing: the authenticated actor selects a private view. */
export async function read_prompt(
  config: Deployment,
  actor: string,
  artifact: ArtifactRef | undefined,
) {
  try {
    const digest = hex_digest(artifact?.sha256?.value ?? new Uint8Array());
    const media = artifact?.mediaType ?? "";
    if (
      !config.prompts ||
      !artifact ||
      !/^[a-f0-9]{64}$/.test(digest) ||
      artifact.uri !== `loop-prompt://sha256/${digest}` ||
      artifact.artifactId?.value !== digest ||
      !prompt_media.some((value) => value === media) ||
      artifact.byteSize < 1n ||
      artifact.byteSize > 4_194_304n ||
      artifact.schema?.name !== "loop.prompt-artifact" ||
      artifact.schema.version !== 1 ||
      !Buffer.from(prompt_schema(media)).equals(
        artifact.schema.schemaSha256?.value ?? new Uint8Array(),
      ) ||
      artifact.rowCount !== undefined ||
      artifact.manifestSha256 ||
      !artifact.createdAt ||
      timestampDate(artifact.createdAt).getTime() > Date.now()
    )
      access_denied();
    const directory = join(config.prompts, actor_directory(actor));
    await private_directory(config.prompts);
    await private_directory(directory);
    const bytes = await read_private(join(directory, digest), 4_194_304);
    if (BigInt(bytes.length) !== artifact.byteSize || hex_digest(json_digest(bytes)) !== digest)
      access_denied();
    if (media === "application/json") parse_json(bytes);
    if (
      media.startsWith("text/") &&
      (!new TextDecoder("utf-8", { fatal: true }).decode(bytes).isWellFormed() ||
        bytes.length > 262_144)
    )
      access_denied();
    if (
      media === "image/png" &&
      !bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
    )
      access_denied();
    if (
      media === "image/jpeg" &&
      (bytes[0] !== 255 || bytes[1] !== 216 || bytes.at(-2) !== 255 || bytes.at(-1) !== 217)
    )
      access_denied();
    if (media === "application/pdf" && !bytes.subarray(0, 5).equals(Buffer.from("%PDF-")))
      access_denied();
    return { media, bytes };
  } catch {
    access_denied();
  }
}

/** Keep supplier continuation bytes off the wire; publish before the response. */
export async function save_continuation(
  config: Deployment,
  actor: string,
  model: ModelResolutionSnapshot,
  text: string,
  state: JsonValue,
) {
  const id = randomUUID();
  const expiry = new Date(Date.now() + 86_400_000);
  const digest = json_digest(json_bytes(state));
  const bytes = json_bytes({
    schema: "loop.provider-continuation/v1",
    actor,
    provider: model.providerId?.value ?? "",
    resolution: model.resolutionId?.value ?? "",
    expires_at: expiry.getTime(),
    text,
    state,
  });
  if (bytes.length > 131_072)
    throw new ProviderError(
      "provider_continuation_limit",
      Code.ResourceExhausted,
      ErrorCategory.DEPENDENCY,
    );
  await finish_invocation(
    config.journal,
    { result_path: join(config.journal, `continuation-${id}.result`) },
    bytes,
  );
  return create(ReasoningContinuationReferenceSchema, {
    providerContinuationId: { value: id },
    providerId: model.providerId,
    modelResolutionId: model.resolutionId,
    stateSha256: { value: digest },
    expiresAt: timestampFromDate(expiry),
  });
}

export async function read_continuation(
  config: Deployment,
  actor: string,
  model: ModelResolutionSnapshot,
  content: ReasoningContent,
): Promise<JsonValue> {
  try {
    const ref = content.continuation;
    const id = ref?.providerContinuationId?.value ?? "";
    if (
      !ref ||
      !/^[a-f0-9-]{36}$/.test(id) ||
      ref.providerId?.value !== model.providerId?.value ||
      ref.modelResolutionId?.value !== model.resolutionId?.value ||
      !ref.expiresAt ||
      timestampDate(ref.expiresAt).getTime() <= Date.now()
    )
      access_denied();
    const outer = parse_json(await read_private(join(config.journal, `continuation-${id}.result`)));
    if (
      outer === null ||
      typeof outer !== "object" ||
      Array.isArray(outer) ||
      typeof outer.bytes !== "string" ||
      typeof outer.sha256 !== "string"
    )
      access_denied();
    const bytes = Buffer.from(outer.bytes, "base64");
    if (bytes.toString("base64") !== outer.bytes || hex_digest(json_digest(bytes)) !== outer.sha256)
      access_denied();
    const record = parse_json(bytes);
    if (
      record === null ||
      typeof record !== "object" ||
      Array.isArray(record) ||
      record.schema !== "loop.provider-continuation/v1" ||
      record.actor !== actor ||
      record.provider !== model.providerId?.value ||
      record.resolution !== model.resolutionId?.value ||
      record.expires_at !== timestampDate(ref.expiresAt).getTime() ||
      record.text !== content.text ||
      record.state === undefined ||
      !Buffer.from(json_digest(json_bytes(record.state))).equals(
        ref.stateSha256?.value ?? new Uint8Array(),
      )
    )
      access_denied();
    return record.state;
  } catch {
    access_denied();
  }
}
