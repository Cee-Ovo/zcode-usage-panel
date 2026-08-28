#!/usr/bin/env node
/**
 * Ensures src-tauri/icons contains a full icon set before a Tauri build.
 * `tauri build` hard-fails when bundle icons are missing, and icons are
 * binary artifacts we don't commit — so every build path funnels through
 * here (predev / prebuild npm hooks, CI).
 */

import { existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const iconIco = join(root, "src-tauri", "icons", "icon.ico");
const sourcePng = join(root, "src-tauri", "icons", "source.png");

if (existsSync(iconIco)) {
  process.exit(0);
}

if (!existsSync(sourcePng)) {
  const gen = spawnSync(process.execPath, [join(__dirname, "gen-icon.mjs")], {
    stdio: "inherit",
  });
  if (gen.status !== 0) {
    console.error("gen-icon.mjs failed");
    process.exit(1);
  }
}

console.log("[ensure-icons] generating Tauri icon set (one-time)…");
const res = spawnSync(
  process.platform === "win32" ? "npx.cmd" : "npx",
  ["tauri", "icon", sourcePng],
  { cwd: root, stdio: "inherit", shell: process.platform === "win32" },
);
if (res.status !== 0) {
  console.error("`npx tauri icon` failed — is @tauri-apps/cli installed?");
  process.exit(1);
}
