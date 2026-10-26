import { test, expect } from "../fixtures/auth.js";
import { cycle, envelope, totals } from "../fixtures/test-data.js";
test("requisições simultâneas disputam saldo sem exceder a renda", async ({
  alice,
}) => {
  const budget = await cycle(alice, 10000);
  const base = `/api/cycles/${budget.id}/envelopes`;
  const results = await Promise.all(
    Array.from({ length: 10 }, (_, i) =>
      alice.api.send("POST", base, {
        name: `Concurrent ${i}`,
        allocated_cents: 2000,
      }),
    ),
  );
  expect(results.filter((r) => r.status() === 201)).toHaveLength(5);
  expect(results.filter((r) => r.status() === 409)).toHaveLength(5);
  expect((await totals(alice, budget.id)).available_cents).toBe(0);
});
test("alterações simultâneas recalculam a alocação atual", async ({
  alice,
}) => {
  const budget = await cycle(alice, 10000);
  const a = await envelope(alice, budget.id, "A", 0);
  const b = await envelope(alice, budget.id, "B", 0);
  const base = `/api/cycles/${budget.id}/envelopes`;
  const results = await Promise.all(
    [a, b].map((e) =>
      alice.api.send("PATCH", `${base}/${e.id}`, { allocated_cents: 6000 }),
    ),
  );
  expect(results.map((r) => r.status()).sort()).toEqual([200, 409]);
  expect((await totals(alice, budget.id)).allocated_cents).toBe(6000);
});
