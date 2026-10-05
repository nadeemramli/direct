import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { execSync } from "node:child_process";

// Stamp the bundle with the source it was built from (DIR-82). A tree with
// uncommitted changes is stamped dirty; without git the stamp is unknown.
function git(args: string): string | null {
  try {
    return execSync(`git ${args}`, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return null;
  }
}
const commit = process.env.DIRECT_BUILD_COMMIT || git("rev-parse HEAD") || "unknown";
const status = commit === "unknown" ? null : git("status --porcelain");
const dirty = process.env.DIRECT_BUILD_DIRTY || (status === null ? "unknown" : status === "" ? "false" : "true");
const build = { commit, dirty, built_at: Math.floor(Date.now() / 1000) };

const buildInfo: Plugin = {
  name: "direct-build-info",
  generateBundle() {
    this.emitFile({ type: "asset", fileName: "build-info.json", source: JSON.stringify(build) });
  },
};

export default defineConfig({
  plugins: [svelte(), buildInfo],
  define: { __DIRECT_BUILD__: JSON.stringify(build) },
  clearScreen: false,
});
