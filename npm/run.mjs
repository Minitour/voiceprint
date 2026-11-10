// Run npm/npx portably. On Windows they are .cmd shims that only run through a shell,
// so build one quoted command line (paths with spaces stay intact, and Node doesn't
// warn about passing an argument array to a shell).
import { execFileSync, execSync } from "node:child_process";

// Anything outside this safe set is double-quoted; that includes `^`, which cmd.exe
// would otherwise treat as an escape character ("npm@^11.15").
const quote = (a) => (/^[\w@%+=:,./\\-]+$/.test(a) ? a : `"${a.replace(/"/g, '\\"')}"`);

export function run(cmd, args, opts = {}) {
  const o = { encoding: "utf8", ...opts };
  if (process.platform !== "win32") return execFileSync(cmd, args, o);
  return execSync([cmd, ...args].map(quote).join(" "), o);
}
