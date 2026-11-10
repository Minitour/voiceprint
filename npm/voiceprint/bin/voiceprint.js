#!/usr/bin/env node
// Launcher for the native voiceprint binary. npm installs exactly one of the
// @voiceprint/<platform>-<arch> packages (as an optional dependency); this script
// finds it and runs it with the same arguments, stdio and exit code.
"use strict";

const { spawnSync } = require("node:child_process");

const SUPPORTED = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-arm64", "win32-x64"];
const key = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "voiceprint.exe" : "voiceprint";

function findBinary() {
  if (process.env.VOICEPRINT_BINARY) return process.env.VOICEPRINT_BINARY;
  try {
    return require.resolve(`@voiceprint/${key}/bin/${exe}`);
  } catch {
    const hint = SUPPORTED.includes(key)
      ? `The optional dependency @voiceprint/${key} is missing. Reinstall without --no-optional / --omit=optional.`
      : `There is no prebuilt binary for ${key} (supported: ${SUPPORTED.join(", ")}).`;
    console.error(
      `voiceprint: ${hint}\n` +
        "You can also build it from source: cargo install --git https://github.com/Minitour/voiceprint\n" +
        "and point VOICEPRINT_BINARY at the result."
    );
    process.exit(1);
  }
}

const result = spawnSync(findBinary(), process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`voiceprint: could not run the binary: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
