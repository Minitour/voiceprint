// Assemble the npm packages from built binaries.
//
//   node npm/assemble.mjs <artifacts-dir> <out-dir>
//
// <artifacts-dir>/<rust-target>/voiceprint[.exe] for each target that was built.
// Writes <out-dir>/voiceprint (the launcher package) and <out-dir>/<platform> for each
// platform package, all at the version in Cargo.toml.
import { copyFileSync, chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const [artifacts, out] = process.argv.slice(2);
if (!artifacts || !out) {
  console.error("usage: node npm/assemble.mjs <artifacts-dir> <out-dir>");
  process.exit(2);
}

const version = readFileSync(join(root, "Cargo.toml"), "utf8").match(/^version\s*=\s*"([^"]+)"/m)[1];
const repository = { type: "git", url: "git+https://github.com/Minitour/voiceprint.git" };
const common = {
  version,
  license: "MIT",
  repository,
  homepage: "https://github.com/Minitour/voiceprint#readme",
  bugs: "https://github.com/Minitour/voiceprint/issues",
};

const PLATFORMS = [
  { target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64" },
  { target: "x86_64-apple-darwin", os: "darwin", cpu: "x64" },
  { target: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64" },
  { target: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64" },
  { target: "aarch64-pc-windows-msvc", os: "win32", cpu: "arm64" },
  { target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64" },
];

rmSync(out, { recursive: true, force: true });
const built = [];
for (const p of PLATFORMS) {
  const exe = p.os === "win32" ? "voiceprint.exe" : "voiceprint";
  const src = join(artifacts, p.target, exe);
  if (!existsSync(src)) {
    console.warn(`skip ${p.os}-${p.cpu}: no binary at ${src}`);
    continue;
  }
  const name = `@voiceprint/${p.os}-${p.cpu}`;
  const dir = join(out, `${p.os}-${p.cpu}`);
  mkdirSync(join(dir, "bin"), { recursive: true });
  copyFileSync(src, join(dir, "bin", exe));
  if (p.os !== "win32") chmodSync(join(dir, "bin", exe), 0o755);
  copyFileSync(join(root, "LICENSE"), join(dir, "LICENSE"));
  copyFileSync(join(root, "data", "NOTICE.md"), join(dir, "NOTICE.md"));
  writeFileSync(
    join(dir, "README.md"),
    `# ${name}\n\nThe ${p.os}-${p.cpu} binary for [voiceprint](https://www.npmjs.com/package/voiceprint). ` +
      "Install `voiceprint` instead; it pulls in the right binary automatically.\n"
  );
  writeFileSync(
    join(dir, "package.json"),
    JSON.stringify(
      {
        name,
        ...common,
        description: `voiceprint binary for ${p.os}-${p.cpu}`,
        os: [p.os],
        cpu: [p.cpu],
        files: ["bin/", "LICENSE", "NOTICE.md", "README.md"],
        preferUnplugged: true,
      },
      null,
      2
    ) + "\n"
  );
  built.push(name);
  console.log(`assembled ${name}@${version}`);
}
if (built.length === 0) {
  console.error("no binaries found");
  process.exit(1);
}

// The launcher package lists every platform, so a release must include all of them.
const main = join(out, "voiceprint");
mkdirSync(join(main, "bin"), { recursive: true });
copyFileSync(join(root, "npm", "voiceprint", "bin", "voiceprint.js"), join(main, "bin", "voiceprint.js"));
copyFileSync(join(root, "LICENSE"), join(main, "LICENSE"));
copyFileSync(join(root, "data", "NOTICE.md"), join(main, "NOTICE.md"));
// npmjs.com can't resolve repo-relative images, so point them at GitHub.
const readme = readFileSync(join(root, "README.md"), "utf8").replace(
  /\]\((docs\/[^)]+)\)/g,
  "](https://raw.githubusercontent.com/Minitour/voiceprint/main/$1)"
);
writeFileSync(join(main, "README.md"), readme);
writeFileSync(
  join(main, "package.json"),
  JSON.stringify(
    {
      name: "voiceprint",
      ...common,
      description: "Let agents write like you: a personal n-gram model that flags words in a draft you wouldn't have written",
      keywords: ["writing", "style", "voice", "llm", "agents", "claude", "n-gram", "cli"],
      bin: { voiceprint: "bin/voiceprint.js" },
      files: ["bin/", "LICENSE", "NOTICE.md", "README.md"],
      engines: { node: ">=16" },
      optionalDependencies: Object.fromEntries(PLATFORMS.map((p) => [`@voiceprint/${p.os}-${p.cpu}`, version])),
    },
    null,
    2
  ) + "\n"
);
console.log(`assembled voiceprint@${version} (${built.length}/${PLATFORMS.length} platforms built)`);
