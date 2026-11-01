import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { loadState, home } from "./runner.mjs";
export function mcpArgs(state) {
  return [
    path.join(home, "node_modules/@playwright/mcp/cli.js"),
    "--headless",
    "--isolated",
    "--browser",
    "chrome",
    "--executable-path",
    chromium.executablePath(),
    "--codegen",
    "none",
    "--console-level",
    "error",
    "--allowed-origins",
    `${state.origin};${new URL(state.issuer).origin};${state.mailpit}`,
    "--output-dir",
    path.join(state.dir, "mcp-private"),
  ];
}
export function serve() {
  const state = loadState();
  const child = spawn(process.execPath, mcpArgs(state), {
    cwd: home,
    env: process.env,
    stdio: "inherit",
  });
  for (const signal of ["SIGINT", "SIGTERM"])
    process.on(signal, () => child.kill(signal));
  child.once("exit", (code) => {
    process.exitCode = code || 0;
  });
}
if (
  process.argv[1] &&
  fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
)
  serve();
