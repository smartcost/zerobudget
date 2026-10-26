import type {
  Reporter,
  TestCase,
  TestResult,
  FullResult,
} from "@playwright/test/reporter";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { state } from "./fixtures/environment.js";
const escape = (value: string) =>
  value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
export default class SafeJUnit implements Reporter {
  private rows: string[] = [];
  private failures = 0;
  onTestEnd(test: TestCase, result: TestResult) {
    const failed = !["passed", "skipped"].includes(result.status);
    if (failed) this.failures++;
    // CI artifact contains no raw error bodies, request headers, snapshots or auth state.
    this.rows.push(
      `<testcase name="${escape(test.titlePath().join(" / "))}" time="${result.duration / 1000}">${failed ? '<failure message="Falha E2E; reproduza o cenário localmente com ./scripts/e2e test."/>' : result.status === "skipped" ? "<skipped/>" : ""}</testcase>`,
    );
  }
  onError(_error: unknown) {
    this.failures++;
    this.rows.push(
      '<testcase name="Runner/bootstrap"><failure message="Falha de descoberta ou preparação do runner E2E."/></testcase>',
    );
  }
  onEnd(_result: FullResult) {
    writeFileSync(
      path.join(state().dir, "junit.xml"),
      `<?xml version="1.0" encoding="UTF-8"?><testsuite name="ZeroBudget E2E" tests="${this.rows.length}" failures="${this.failures}">${this.rows.join("")}</testsuite>`,
    );
  }
}
