import { test, expect } from "../fixtures/auth.js";
import { body, rejection, type Cycle } from "../fixtures/api-client.js";
import { cycle, totals } from "../fixtures/test-data.js";
test("cria, lista e consulta ciclo conforme OpenAPI", async ({ alice }) => {
  const budget = await cycle(alice, 500000);
  expect(budget.allocated_cents).toBe(0);
  expect(budget.available_cents).toBe(500000);
  expect(budget.status).toBe("planned");
  expect(budget.currency).toBe("BRL");
  expect((await totals(alice, budget.id)).id).toBe(budget.id);
  const listed = await alice.api.send("GET", "/api/cycles?limit=100&offset=0");
  expect(listed.status()).toBe(200);
  for (const value of await listed.json())
    expect(typeof value.id).toBe("string");
  await rejection(
    await alice.api.send("POST", "/api/cycles", {
      period_start: budget.period_start,
      period_end: budget.period_end,
      income_cents: 500000,
    }),
    409,
  );
});
test("valida períodos, valores e proprietário proibido", async ({ alice }) => {
  for (const input of [
    { period_start: "2026-02-30", period_end: "2026-03-01", income_cents: 100 },
    { period_start: "2026-03-01", period_end: "2026-02-01", income_cents: 100 },
    { period_start: "2026-03-01", period_end: "2026-04-01", income_cents: -1 },
    {
      period_start: "2026-03-01",
      period_end: "2026-04-01",
      income_cents: 100,
      owner_id: alice.me.user_id,
    },
  ])
    await rejection(await alice.api.send("POST", "/api/cycles", input), 422);
});
