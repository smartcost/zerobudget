import { test, expect } from "../fixtures/auth.js";
import { rejection } from "../fixtures/api-client.js";
import { admin } from "../fixtures/environment.js";
import { cycle } from "../fixtures/test-data.js";
test("nega por padrão sem permissões financeiras", async ({ login }) => {
  const actor = await login("denied");
  await rejection(await actor.api.send("GET", "/api/cycles"), 403);
  await rejection(
    await actor.api.send("POST", "/api/cycles", {
      period_start: "2027-01-01",
      period_end: "2027-02-01",
      income_cents: 100,
    }),
    403,
  );
});
test("somente leitura pode consultar mas não alterar", async ({ login }) => {
  const actor = await login("reader", ["budget:read", "budget:write"]);
  const budget = await cycle(actor);
  await admin(["revoke", actor.me.user_id, "budget:write"]);
  expect(
    (await actor.api.send("GET", `/api/cycles/${budget.id}`)).status(),
  ).toBe(200);
  await rejection(
    await actor.api.send("POST", `/api/cycles/${budget.id}/envelopes`, {
      name: "Denied",
      allocated_cents: 0,
    }),
    403,
  );
});
test("somente escrita cria sem ganhar leitura HTTP", async ({ login }) => {
  const actor = await login("writer");
  const budget = await cycle(actor);
  expect(
    (
      await actor.api.send("POST", `/api/cycles/${budget.id}/envelopes`, {
        name: "Write only",
        allocated_cents: 100,
      })
    ).status(),
  ).toBe(201);
  await rejection(await actor.api.send("GET", `/api/cycles/${budget.id}`), 403);
});
test("revogação e bloqueio valem para sessão existente; logout permanece disponível", async ({
  alice,
}) => {
  await admin(["revoke", alice.me.user_id, "budget:read"]);
  await rejection(await alice.api.send("GET", "/api/cycles"), 403);
  await admin(["disable", alice.me.user_id]);
  await rejection(await alice.api.send("GET", "/api/auth/me"), 403);
  expect((await alice.api.send("POST", "/api/auth/logout")).status()).toBe(204);
});
