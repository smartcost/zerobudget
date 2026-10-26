import { defineConfig } from "@playwright/test";
import { readFileSync } from "node:fs";
import path from "node:path";
const filename = process.env.ZB_E2E_STATE;
if (!filename)
  throw new Error("Use ./scripts/e2e test ou ui para preparar o ambiente.");
const state = JSON.parse(readFileSync(filename, "utf8"));
export default defineConfig({
  testDir: "./specs",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 45000,
  expect: { timeout: 10000 },
  outputDir: path.join(state.dir, "test-results"),
  reporter: [
    ["list"],
    ["html", { outputFolder: path.join(state.dir, "report"), open: "never" }],
    [path.join(import.meta.dirname, "junit-reporter.ts")],
  ],
  use: {
    baseURL: state.origin,
    browserName: "chromium",
    headless: true,
    actionTimeout: 10000,
    navigationTimeout: 15000,
    trace: "off",
    screenshot: "off",
    video: "off",
  },
});
