import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, "dist");
const apiBaseUrl = process.env.ARXIVIST_API_BASE_URL ?? "";
const agentApiBaseUrl = process.env.ARXIVIST_AGENT_API_BASE_URL ?? "";

await mkdir(dist, { recursive: true });
await writeFile(
  join(dist, "config.js"),
  `window.ARXIVIST_CONFIG = ${JSON.stringify({ apiBaseUrl, agentApiBaseUrl })};\n`
);
