import {
  expect,
  type APIRequestContext,
  type APIResponse,
} from "@playwright/test";
import { readFileSync } from "node:fs";
import path from "node:path";
import Ajv from "ajv";
import addFormats from "ajv-formats";
import { repo } from "./environment.js";
const spec = JSON.parse(
  readFileSync(path.join(repo, "docs/api/openapi.json"), "utf8"),
);
const ajv = new Ajv.default({
  strict: false,
  allErrors: true,
  formats: { int64: true },
});
addFormats.default(ajv);
ajv.addSchema({ $id: "zerobudget", components: spec.components });
export type Cycle = {
  id: string;
  period_start: string;
  period_end: string;
  income_cents: number;
  allocated_cents: number;
  available_cents: number;
  status: string;
  currency: string;
};
export type Envelope = {
  id: string;
  cycle_id: string;
  name: string;
  allocated_cents: number;
  spent_cents: number;
  remaining_cents: number;
};
export type Expense = {
  account_id: string | null;
  id: string;
  cycle_id: string;
  envelope_id: string;
  description: string;
  occurred_on: string;
  amount_cents: number;
  status: "posted" | "reversed";
};
export class ApiClient {
  constructor(
    readonly request: APIRequestContext,
    private csrf: string,
  ) {}
  setCsrf(value: string) {
    this.csrf = value;
  }
  async send(
    method: string,
    url: string,
    data?: unknown,
    options: { csrf?: boolean; origin?: string } = {},
  ): Promise<APIResponse> {
    const headers: Record<string, string> = {};
    if (!["GET", "HEAD", "OPTIONS"].includes(method) && options.csrf !== false)
      headers["X-CSRF-Token"] = this.csrf;
    if (options.origin) headers.Origin = options.origin;
    try {
      return await this.request.fetch(url, {
        method,
        data,
        headers,
        failOnStatusCode: false,
        maxRetries: 0,
      });
    } catch {
      throw new Error(
        `Falha de transporte em ${method} ${url}; mutação não foi repetida.`,
      );
    }
  }
}
export async function body<T>(
  response: APIResponse,
  status: number,
  schema: string,
): Promise<T> {
  // Do not attach raw auth/HTTP response bodies or headers to reports.
  expect(response.status(), `HTTP de ${new URL(response.url()).pathname}`).toBe(
    status,
  );
  expect(response.headers()["content-type"]).toContain("application/json");
  const value = await response.json();
  const validate =
    ajv.getSchema(`zerobudget#/components/schemas/${schema}`) ||
    ajv.compile({ $ref: `zerobudget#/components/schemas/${schema}` });
  expect(Boolean(validate(value)), `Contrato ${schema} inválido`).toBe(true);
  return value as T;
}
export async function rejection(
  response: APIResponse,
  status: number,
): Promise<void> {
  expect(response.status()).toBe(status);
  const type = response.headers()["content-type"] || "";
  if (type.includes("application/json")) await body(response, status, "Error");
  else {
    expect(type).toContain("text/plain");
    expect((await response.text()).length).toBeGreaterThan(0);
  }
}
