import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { state } from "./environment.js";
import { body, type Cycle, type Envelope } from "./api-client.js";
import type { Actor } from "./auth.js";
export async function cycle(actor: Actor, income = 10000): Promise<Cycle> {
  // One worker; counter is persisted per isolated run so UI reruns cannot reuse periods.
  const filename = path.join(state().dir, "period-counter.json");
  const index = existsSync(filename)
    ? Number(readFileSync(filename, "utf8"))
    : 0;
  writeFileSync(filename, String(index + 1));
  const start = new Date(Date.UTC(2100, 0, index + 1));
  const end = new Date(Date.UTC(2100, 0, index + 2));
  return body<Cycle>(
    await actor.api.send("POST", "/api/cycles", {
      period_start: start.toISOString().slice(0, 10),
      period_end: end.toISOString().slice(0, 10),
      income_cents: income,
    }),
    201,
    "Cycle",
  );
}
export async function envelope(
  actor: Actor,
  cycleId: string,
  name: string,
  amount: number,
): Promise<Envelope> {
  return body<Envelope>(
    await actor.api.send("POST", `/api/cycles/${cycleId}/envelopes`, {
      name,
      allocated_cents: amount,
    }),
    201,
    "Envelope",
  );
}
export async function totals(actor: Actor, id: string): Promise<Cycle> {
  return body<Cycle>(
    await actor.api.send("GET", `/api/cycles/${id}`),
    200,
    "Cycle",
  );
}
