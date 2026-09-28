import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { type FileHandle, link, mkdir, open, unlink } from "node:fs/promises";
import { dirname, join } from "node:path";

const MAX_RECORD_BYTES = 1_048_576;

/** A prior outbound attempt may have incurred cost; never silently resend it. */
export class JournalError extends Error {
  readonly code: "invocation_conflict" | "invocation_ambiguous" | "journal_unavailable";

  constructor(code: "invocation_conflict" | "invocation_ambiguous" | "journal_unavailable") {
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

/** Private provider state only. No research store, prompt or secret is a claim. */
export async function open_journal(directory: string): Promise<void> {
  // Require the administrative parent to exist. Persist this entry before any
  // outbound work so a process/power interruption cannot discard the journal.
  await mkdir(directory, { mode: 0o700 }).catch((error: unknown) => {
    if (error_code(error) !== "EEXIST") throw new JournalError("journal_unavailable");
  });
  const handle = await open(
    directory,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    const info = await handle.stat();
    if ((info.mode & 0o077) !== 0 || info.uid !== process.getuid?.()) {
      throw new JournalError("journal_unavailable");
    }
  } finally {
    await handle.close();
  }
  await sync_directory(directory);
  await sync_directory(dirname(directory));
}

async function read_record(path: string): Promise<Uint8Array> {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (
      !info.isFile() ||
      info.size < 1 ||
      info.size > MAX_RECORD_BYTES ||
      (info.mode & 0o077) !== 0
    ) {
      throw new JournalError("journal_unavailable");
    }
    const bytes = await handle.readFile();
    if (bytes.length !== info.size) throw new JournalError("journal_unavailable");
    return bytes;
  } finally {
    await handle.close();
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
