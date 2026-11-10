// One-time setup: make GitHub Actions (release.yml in this repo) the trusted publisher
// for every voiceprint package, so releases need no npm token.
//
//   node npm/trust.mjs
//
// Run it after `npm login` and after the packages exist on npm. `npm trust` needs
// npm >= 11.15 and 2FA on your npm account; this script runs a current npm through
// npx, so the npm you have installed doesn't matter.
import { run } from "./run.mjs";

const REPO = "Minitour/voiceprint";
const WORKFLOW = "release.yml";
const NPM = "npm@^11.15"; // has `npm trust`, and still runs on Node 20 and 22+
const PACKAGES = [
  "voiceprint",
  "@voiceprint/darwin-arm64",
  "@voiceprint/darwin-x64",
  "@voiceprint/linux-arm64",
  "@voiceprint/linux-x64",
  "@voiceprint/win32-arm64",
  "@voiceprint/win32-x64",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failed = [];

for (const pkg of PACKAGES) {
  console.log(`\ntrust ${pkg} -> github.com/${REPO} (${WORKFLOW})`);
  try {
    run("npx", ["-y", NPM, "trust", "github", pkg, "--file", WORKFLOW, "--repo", REPO, "--allow-publish", "--yes"], {
      stdio: "inherit",
    });
  } catch {
    failed.push(pkg);
  }
  await sleep(2000); // npm asks for a short pause between trust calls
}

if (failed.length) {
  console.error(`\nNot configured: ${failed.join(", ")}. Fix the error above and run this again.`);
  process.exit(1);
}
console.log(`\nDone. Check with: npx -y ${NPM} trust list voiceprint`);
