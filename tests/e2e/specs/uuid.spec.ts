import { test, expect } from "../fixtures/auth.js";
import { body, rejection } from "../fixtures/api-client.js";
import { account, cycle, envelope } from "../fixtures/test-data.js";
import { uuidv7 } from "../fixtures/uuid.js";

const pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
test("UUIDv7: geração local, rejeição de v4/variante inválida e retry idempotente", async ({ alice }) => {
  const a = await account(alice, 0);
  const c = await cycle(alice);
  const e = await envelope(alice, c.id, "UUIDv7", 100);
  for (const id of [a.id, c.id, e.id]) expect(id).toMatch(pattern);
  const requestId = uuidv7();
  expect(requestId).toMatch(pattern);
  expect(Math.abs(parseInt(requestId.replaceAll("-", "").slice(0, 12), 16) - Date.now())).toBeLessThan(5000);
  const route = `/api/accounts/${a.id}/receipts`;
  const payload = { request_id: requestId, description: "UUIDv7", occurred_on: "2026-10-01", amount_cents: 10 };
  for (const invalid of ["550e8400-e29b-41d4-a716-446655440000", "019a0000-0000-7000-0000-000000000001"]) {
    await rejection(await alice.api.send("POST", route, { ...payload, request_id: invalid }), 422);
  }
  const first = await body<{id: string}>(await alice.api.send("POST", route, payload), 201, "Movement");
  const retry = await body<{id: string}>(await alice.api.send("POST", route, payload), 201, "Movement");
  expect(first.id).toBe(requestId);
  expect(retry).toEqual(first);
  await rejection(await alice.api.send("POST", route, { ...payload, amount_cents: 11 }), 409);
  const balance = await body<{balance_cents: number}>(await alice.api.send("GET", `/api/accounts/${a.id}`), 200, "Account");
  expect(balance.balance_cents).toBe(10);
});
