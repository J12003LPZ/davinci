// Build-time only. Runtime never installs packages or trusts an adjacent manifest.
import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, copyFile, writeFile } from "node:fs/promises";
import { resolve, relative, join } from "node:path";
import { fileURLToPath } from "node:url";

const source = fileURLToPath(new URL(".", import.meta.url));
const destination = process.argv[2];
const manifestPath = process.argv[3];
if (!destination || !manifestPath) throw new Error("usage: package-bundle.mjs NEW_DIRECTORY MANIFEST_PATH");
const root = resolve(destination);
// Never overwrite an existing directory, including somebody else's installation.
await mkdir(root, { recursive: false });
const paths = ["host.mjs", "framing.mjs", "bounded-worker.mjs", "strict-prelude.mjs"];
async function collect(directory, accept) {
  for (const entry of await readdir(join(source, directory), { withFileTypes: true })) {
    const path = `${directory}/${entry.name}`;
    if (entry.isSymbolicLink()) throw new Error("linked source asset");
    if (entry.isDirectory()) await collect(path, accept);
    else if (accept(path)) paths.push(path);
  }
}
await collect("licenses", () => true);
for (const name of ["@earendil-works/pi-codemode", "quickjs-wasi"]) {
  paths.push(`node_modules/${name}/package.json`);
  await collect(`node_modules/${name}/dist`, (path) => path.endsWith(".js"));
}
paths.push("node_modules/quickjs-wasi/quickjs.wasm");
const assets = [];
for (const path of paths.sort()) {
  const target = join(root, path);
  await mkdir(resolve(target, ".."), { recursive: true });
  await copyFile(join(source, path), target);
  const bytes = await readFile(target);
  assets.push({ path, bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
}
const manifest = {
  schema_version: 1, protocol_version: 1, node_version: "24.21.0",
  pi_version: "1.0.2", quickjs_version: "3.6.2", entry: "host.mjs",
  worker: "bounded-worker.mjs", wasm: "node_modules/quickjs-wasi/quickjs.wasm", assets,
};
if (relative(root, resolve(manifestPath)).split(/[\\/]/)[0] !== "..") {
  throw new Error("trusted manifest must be emitted outside the runtime bundle");
}
await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
console.log(`packaged ${assets.length} trusted assets`);
