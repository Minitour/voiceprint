// Fail if any plugin or marketplace manifest disagrees with the version in Cargo.toml,
// so the agent plugin and the CLI always release together.
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(`../../${p}`, import.meta.url), "utf8");
const want = read("Cargo.toml").match(/^version\s*=\s*"([^"]+)"/m)[1];

const found = {
  "plugins/voiceprint/plugin.json": JSON.parse(read("plugins/voiceprint/plugin.json")).version,
  "plugins/voiceprint/.claude-plugin/plugin.json": JSON.parse(read("plugins/voiceprint/.claude-plugin/plugin.json")).version,
  "plugins/voiceprint/.codex-plugin/plugin.json": JSON.parse(read("plugins/voiceprint/.codex-plugin/plugin.json")).version,
  "plugins/voiceprint/.cursor-plugin/plugin.json": JSON.parse(read("plugins/voiceprint/.cursor-plugin/plugin.json")).version,
  ".claude-plugin/marketplace.json": JSON.parse(read(".claude-plugin/marketplace.json")).plugins[0].version,
};
// The Cursor and Codex marketplaces carry no version; just make sure they parse.
JSON.parse(read(".cursor-plugin/marketplace.json"));
JSON.parse(read(".agents/plugins/marketplace.json"));

const wrong = Object.entries(found).filter(([, v]) => v !== want);
for (const [file, v] of Object.entries(found)) console.log(`${v === want ? "ok " : "BAD"} ${file}: ${v}`);
if (wrong.length) {
  console.error(`\nCargo.toml is ${want}; update the files marked BAD.`);
  process.exit(1);
}
