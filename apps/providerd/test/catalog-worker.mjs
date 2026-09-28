import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";

const [config_path, candidate_path, mode] = process.argv.slice(2);
if (mode === "prepared") {
  // Pause the real publisher after fsync and before its exclusive link.
  fs.promises.link = async () => {
    process.send?.("prepared");
    setInterval(() => {}, 1000);
    await new Promise(() => {});
  };
  syncBuiltinESMExports();
}
const { load_deployment } = await import("../dist/config.js");
const { publish_catalog } = await import("../dist/catalog-store.js");
const config = await load_deployment(config_path);
const candidate = JSON.parse(await fs.promises.readFile(candidate_path, "utf8"));
process.send?.("ready");
process.once("message", async () => {
  try {
    await publish_catalog(config, candidate);
    process.stdout.write("published\n");
    if (mode === "completed") {
      process.send?.("completed");
      setInterval(() => {}, 1000);
      return;
    }
  } catch (error) {
    process.stdout.write(`${error.code ?? "failed"}\n`);
  }
  process.disconnect();
});
