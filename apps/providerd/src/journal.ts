import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { type FileHandle, link, mkdir, open, unlink } from "node:fs/promises";
import { dirname, join } from "node:path";

const MAX_RECORD_BYTES = 1_048_576;

/** A prior outbound attempt may have incurred cost; never silently resend it. */
export class JournalError extends Error {
  readonly code:
    | "invocation_conflict"
    | "invocation_ambiguous"
    | "journal_unavailable"
    | "provider_receipt_corrupt";

  constructor(code: JournalError["code"]) {
    super(code);
    this.code = code;
    this.name = "JournalError";
  }
}

export interface InvocationClaim {
  readonly schema: "loop.provider-claim/v1";
  readonly actor: string;
  readonly request_sha256: string;
  readonly reserved_nano_usd: string;
}

export interface InvocationSlot {
  readonly result_path: string;
  readonly cached?: Uint8Array;
}

/** Open private Provider storage. With create=false, missing storage fails
 * closed and validation performs no mkdir or directory sync writes.
 */
export async function open_journal(directory: string, create = true): Promise<void> {
  // Require the administrative parent to exist. Persist this entry before any
  // outbound work so a process/power interruption cannot discard the journal.
  if (create)
    await mkdir(directory, { mode: 0o700 }).catch((error: unknown) => {
      if (error_code(error) !== "EEXIST") throw new JournalError("journal_unavailable");
    });
  const handle = await open_directory(directory);
  await handle.close();
  if (create) {
    await sync_directory(directory);
    await sync_directory(dirname(directory));
  }
}

async function open_directory(directory: string, expected?: FileHandle): Promise<FileHandle> {
  let handle: FileHandle | undefined;
  try {
    handle = await open(
      directory,
      constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
    );
    const info = await handle.stat();
    const previous = await expected?.stat();
    if (
      (info.mode & 0o077) !== 0 ||
      info.uid !== process.getuid?.() ||
      (previous && (previous.dev !== info.dev || previous.ino !== info.ino))
    )
      throw new JournalError("journal_unavailable");
    return handle;
  } catch {
    await handle?.close();
    throw new JournalError("journal_unavailable");
  }
}

async function read_record(path: string, partial?: false): Promise<Uint8Array>;
async function read_record(path: string, partial: true): Promise<Uint8Array | undefined>;
async function read_record(path: string, partial = false): Promise<Uint8Array | undefined> {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const info = await handle.stat();
    if (
      !info.isFile() ||
      info.size < (partial ? 0 : 1) ||
      info.size > MAX_RECORD_BYTES ||
      info.uid !== process.getuid?.() ||
      (info.mode & 0o077) !== 0
    ) {
      throw new JournalError("journal_unavailable");
    }
    // The extra byte detects growth without allowing a later stat/readFile to
    // allocate an arbitrarily enlarged record. Both memory and reads are bounded.
    const bytes = Buffer.alloc(info.size + 1);
    let length = 0;
    while (length < bytes.length) {
      const read = await handle.read(bytes, length, bytes.length - length, length);
      if (read.bytesRead === 0) break;
      length += read.bytesRead;
    }
    const current = await handle.stat();
    if (length !== info.size || current.size !== info.size) {
      // Exclusive claims are visible before their complete bytes are written.
      // A growing claim cannot yet establish a trustworthy reservation.
      if (partial && current.size >= info.size && length >= info.size) return undefined;
      throw new JournalError("journal_unavailable");
    }
    return bytes.subarray(0, length);
  } finally {
    await handle.close();
  }
}

export type InvocationRecord =
  | { readonly state: "absent" }
  | { readonly state: "ambiguous"; readonly reserved?: bigint }
  | { readonly state: "completed"; readonly reserved: bigint; readonly bytes: Uint8Array };

/** Read an actor's immutable receipt without claiming a key or outbound work.
 * Absence is an observation, never proof that no request is still in flight.
 * A partial exclusive claim remains ambiguous; malformed completed evidence
 * fails closed. No catalog, current model or supplier credential is required.
 */
export async function read_invocation(
  directory: string,
  key: string,
  actor: string,
  request_sha256: string,
): Promise<InvocationRecord> {
  // Missing/replaced storage must not look like a missing invocation. Never
  // create the journal during a lookup and never follow a directory symlink.
  const root = await open_directory(directory);
  try {
    // Linux procfs provides directory-relative access through the retained FD.
    // O_NOFOLLOW on each record still rejects record symlinks. The configured
    // parent remains trusted; replacement/removal of its journal fails closed.
    const directory_fd = `/proc/self/fd/${root.fd}`;
    // A missing procfs mount is unavailable storage, never missing evidence.
    const attached = await open_directory(`${directory_fd}/.`, root);
    await attached.close();
    return await read_evidence(directory_fd, key, actor, request_sha256);
  } finally {
    try {
      const current = await open_directory(directory, root);
      await current.close();
    } finally {
      await root.close();
    }
  }
}

async function read_evidence(
  directory: string,
  key: string,
  actor: string,
  request_sha256: string,
): Promise<InvocationRecord> {
  const id = createHash("sha256").update(actor).update("\0").update(key).digest("hex");
  let bytes: Uint8Array | undefined;
  let result: Uint8Array | undefined;
  try {
    bytes = await read_record(join(directory, `${id}.claim`), true);
  } catch (error) {
    if (error_code(error) !== "ENOENT") throw new JournalError("provider_receipt_corrupt");
    try {
      result = await read_record(join(directory, `${id}.result`));
    } catch (result_error) {
      if (error_code(result_error) === "ENOENT") return { state: "absent" };
      throw new JournalError("provider_receipt_corrupt");
    }
    // A writer may publish its claim and result after our first missing read.
    // Recheck once before declaring an orphan; this never authorizes a write.
    try {
      bytes = await read_record(join(directory, `${id}.claim`), true);
    } catch {
      throw new JournalError("provider_receipt_corrupt");
    }
  }
  if (!bytes) return { state: "ambiguous" };
  let claim: unknown;
  try {
    claim = JSON.parse(Buffer.from(bytes).toString("utf8"));
  } catch {
    // O_EXCL publication precedes writing claim bytes. A killed writer may
    // leave any prefix; lookup cannot establish its request identity or cost.
    return { state: "ambiguous" };
  }
  if (
    typeof claim !== "object" ||
    claim === null ||
    Object.keys(claim).sort().join(",") !== "actor,request_sha256,reserved_nano_usd,schema" ||
    !("schema" in claim) ||
    claim.schema !== "loop.provider-claim/v1" ||
    !("actor" in claim) ||
    claim.actor !== actor ||
    !("request_sha256" in claim) ||
    typeof claim.request_sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(claim.request_sha256) ||
    !("reserved_nano_usd" in claim) ||
    typeof claim.reserved_nano_usd !== "string" ||
    !/^(?:0|[1-9][0-9]{0,14})$/.test(claim.reserved_nano_usd)
  )
    throw new JournalError("provider_receipt_corrupt");
  if (claim.request_sha256 !== request_sha256) throw new JournalError("invocation_conflict");
  const reserved = BigInt(claim.reserved_nano_usd);
  try {
    result ??= await read_record(join(directory, `${id}.result`));
  } catch (error) {
    if (error_code(error) === "ENOENT") return { state: "ambiguous", reserved };
    throw new JournalError("provider_receipt_corrupt");
  }
  try {
    const record: unknown = JSON.parse(Buffer.from(result).toString("utf8"));
    if (
      typeof record !== "object" ||
      record === null ||
      Object.keys(record).sort().join(",") !== "bytes,sha256" ||
      !("bytes" in record) ||
      typeof record.bytes !== "string" ||
      !("sha256" in record) ||
      typeof record.sha256 !== "string"
    )
      throw new Error("invalid_record");
    const content = Buffer.from(record.bytes, "base64");
    if (
      content.length < 1 ||
      content.length > 524_288 ||
      content.toString("base64") !== record.bytes ||
      createHash("sha256").update(content).digest("hex") !== record.sha256
    )
      throw new Error("invalid_record");
    return { state: "completed", reserved, bytes: content };
  } catch {
    throw new JournalError("provider_receipt_corrupt");
  }
}

function error_code(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error
    ? String(error.code)
    : undefined;
}

/** Exclusive durable claim; a missing final response fences retries after crash. */
export async function claim_invocation(
  directory: string,
  key: string,
  claim: InvocationClaim,
): Promise<InvocationSlot> {
  const id = createHash("sha256").update(claim.actor).update("\0").update(key).digest("hex");
  const claim_path = join(directory, `${id}.claim`);
  const result_path = join(directory, `${id}.result`);
  const expected = JSON.stringify(claim);
  let handle: FileHandle;
  try {
    handle = await open(
      claim_path,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
  } catch (error) {
    if (error_code(error) !== "EEXIST") throw new JournalError("journal_unavailable");
    let recorded: string;
    try {
      recorded = Buffer.from(await read_record(claim_path)).toString("utf8");
    } catch {
      throw new JournalError("invocation_ambiguous");
    }
    if (recorded !== expected) throw new JournalError("invocation_conflict");
    try {
      const record: unknown = JSON.parse(
        Buffer.from(await read_record(result_path)).toString("utf8"),
      );
      if (
        typeof record !== "object" ||
        record === null ||
        !("bytes" in record) ||
        !("sha256" in record) ||
        typeof record.bytes !== "string" ||
        typeof record.sha256 !== "string"
      ) {
        throw new JournalError("journal_unavailable");
      }
      const cached = Buffer.from(record.bytes, "base64");
      if (
        cached.toString("base64") !== record.bytes ||
        createHash("sha256").update(cached).digest("hex") !== record.sha256
      ) {
        throw new JournalError("journal_unavailable");
      }
      return { result_path, cached };
    } catch {
      throw new JournalError("invocation_ambiguous");
    }
  }
  try {
    await handle.writeFile(expected);
    await handle.sync();
    await sync_directory(directory);
  } finally {
    await handle.close();
  }
  return { result_path };
}

async function sync_directory(directory: string): Promise<void> {
  const handle = await open(
    directory,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

/** Publish only after complete output validation. No incomplete result is replayed. */
export async function finish_invocation(
  directory: string,
  slot: InvocationSlot,
  bytes: Uint8Array,
): Promise<void> {
  if (bytes.length < 1 || bytes.length > 524_288 || slot.cached !== undefined) {
    throw new JournalError("journal_unavailable");
  }
  const temporary = join(directory, `.pending-${randomUUID()}`);
  const handle = await open(
    temporary,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL,
    0o600,
  );
  try {
    await handle.writeFile(
      JSON.stringify({
        bytes: Buffer.from(bytes).toString("base64"),
        sha256: createHash("sha256").update(bytes).digest("hex"),
      }),
    );
    await handle.sync();
    await handle.close();
    await link(temporary, slot.result_path);
    await sync_directory(directory);
  } finally {
    await handle.close();
    await unlink(temporary).catch(() => undefined);
  }
}
