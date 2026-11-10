// Publish assembled packages: every platform package first, then the launcher.
//
//   node npm/publish.mjs <dir-from-assemble.mjs> [--dry-run]
//
// Versions already on the registry are skipped, so a half-finished release can be
// re-run. In GitHub Actions this authenticates through trusted publishing (OIDC);
// locally it uses your `npm login` session.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
const dryRun = process.argv.includes("--dry-run");
if (!dir) {
  console.error("usage: node npm/publish.mjs <dir>");
  process.exit(2);
}

const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const run = (args, opts = {}) => execFileSync(npm, args, { encoding: "utf8", shell: process.platform === "win32", ...opts });

const pkgs = readdirSync(dir, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => join(dir, d.name))
  .map((path) => ({ path, ...JSON.parse(readFileSync(join(path, "package.json"), "utf8")) }))
  // The launcher depends on the platform packages, so it goes last.
  .sort((a, b) => (a.name === "voiceprint") - (b.name === "voiceprint") || a.name.localeCompare(b.name));

for (const p of pkgs) {
  let published = "";
  try {
    published = run(["view", `${p.name}@${p.version}`, "version"], { stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    // not published yet (npm view exits non-zero for unknown packages)
  }
  if (published === p.version) {
    console.log(`skip ${p.name}@${p.version} (already published)`);
    continue;
  }
  console.log(`publish ${p.name}@${p.version}`);
  // Publish from inside the package folder: passing its path as an argument breaks on
  // Windows when the path contains spaces (npm.cmd runs through cmd.exe).
  run(["publish", "--access", "public", ...(dryRun ? ["--dry-run"] : [])], { stdio: "inherit", cwd: p.path });
}
