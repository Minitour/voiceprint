// One-time setup: make GitHub Actions (release.yml in this repo) the trusted publisher
// for every voiceprint package, so releases need no npm token.
//
//   node npm/trust.mjs            (after `npm login`; needs npm >= 11.15 and account 2FA)
//
// The packages must already exist on npm, so run this after the first publish.
import { execFileSync } from "node:child_process";

const REPO = "Minitour/voiceprint";
const WORKFLOW = "release.yml";
const PACKAGES = [
  "voiceprint",
  "@voiceprint/darwin-arm64",
  "@voiceprint/darwin-x64",
  "@voiceprint/linux-arm64",
  "@voiceprint/linux-x64",
  "@voiceprint/win32-arm64",
  "@voiceprint/win32-x64",
];

const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const shell = process.platform === "win32";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

for (const pkg of PACKAGES) {
  console.log(`trust ${pkg} -> github.com/${REPO} (${WORKFLOW})`);
  execFileSync(npm, ["trust", "github", pkg, "--file", WORKFLOW, "--repo", REPO, "--allow-publish", "--yes"], {
    stdio: "inherit",
    shell,
  });
  await sleep(2000); // npm asks for a short pause between trust calls
}
console.log("\nDone. Check with: npm trust list voiceprint");
