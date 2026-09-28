import { claim_invocation, finish_invocation, open_journal } from "../src/journal.ts";

const [directory, key, mode] = process.argv.slice(2);
await open_journal(directory);
try {
  const slot = await claim_invocation(directory, key, {
    schema: "loop.provider-claim/v1",
    actor: "test-owner",
    request_sha256: "a".repeat(64),
    reserved_nano_usd: "100",
  });
  if (slot.cached) process.stdout.write("cached\n");
  else if (mode === "race") process.stdout.write("claimed\n");
  else {
    if (mode === "completed")
      await finish_invocation(directory, slot, new TextEncoder().encode("complete response"));
    process.stdout.write(`${mode}\n`);
    setInterval(() => {}, 1000);
  }
} catch (error) {
  process.stdout.write(`${error.code ?? "failed"}\n`);
}
