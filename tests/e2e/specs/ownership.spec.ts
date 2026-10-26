import { test, expect } from "../fixtures/auth.js";
import { rejection } from "../fixtures/api-client.js";
import { cycle, envelope } from "../fixtures/test-data.js";
test("dois usuários reais não acessam ciclos ou envelopes alheios", async ({
  alice,
  login,
}) => {
  const bob = await login("bob");
  expect(bob.me.user_id).not.toBe(alice.me.user_id);
  const budget = await cycle(alice);
  const a = await envelope(alice, budget.id, "Private", 100);
  const base = `/api/cycles/${budget.id}/envelopes`;
  await rejection(await bob.api.send("GET", `/api/cycles/${budget.id}`), 404);
  await rejection(await bob.api.send("GET", base), 404);
  await rejection(
    await bob.api.send("POST", base, { name: "Intruder", allocated_cents: 0 }),
    404,
  );
  await rejection(
    await bob.api.send("PATCH", `${base}/${a.id}`, { allocated_cents: 0 }),
    404,
  );
  await rejection(await bob.api.send("DELETE", `${base}/${a.id}`), 404);
});
test("trocar o ciclo na URL não muda o vínculo do envelope", async ({
  alice,
}) => {
  const first = await cycle(alice);
  const second = await cycle(alice);
  const a = await envelope(alice, first.id, "Fixed", 100);
  const wrong = `/api/cycles/${second.id}/envelopes/${a.id}`;
  await rejection(await alice.api.send("PATCH", wrong, { name: "Moved" }), 404);
  await rejection(await alice.api.send("DELETE", wrong), 404);
});
