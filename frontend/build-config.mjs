import { cp, mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(fileURLToPath(import.meta.url));
const source = join(root, "src");
const dist = join(root, "dist");
const apiBaseUrl = process.env.ARXIVIST_API_BASE_URL ?? "";

await mkdir(dist, { recursive: true });
await cp(source, dist, { recursive: true });
await writeFile(
  join(dist, "config.js"),
  `window.ARXIVIST_CONFIG = ${JSON.stringify({ apiBaseUrl })};\n`
);
