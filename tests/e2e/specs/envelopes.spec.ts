import { test, expect } from "../fixtures/auth.js";
import { body, rejection, type Envelope } from "../fixtures/api-client.js";
import { cycle, envelope, totals } from "../fixtures/test-data.js";
test("CRUD de envelopes distribui e libera saldo", async ({ alice }) => {
  const budget = await cycle(alice, 10000);
  const base = `/api/cycles/${budget.id}/envelopes`;
  const a = await envelope(alice, budget.id, " Moradia ", 6000);
  expect(a.name).toBe("Moradia");
  const b = await envelope(alice, budget.id, "Reserva", 4000);
  expect((await totals(alice, budget.id)).available_cents).toBe(0);
  expect((await alice.api.send("GET", base)).status()).toBe(200);
  const changed = await body<Envelope>(
    await alice.api.send("PATCH", `${base}/${a.id}`, {
      name: "Casa",
      allocated_cents: 2000,
    }),
    200,
    "Envelope",
  );
  expect(changed.name).toBe("Casa");
  expect((await totals(alice, budget.id)).available_cents).toBe(4000);
  expect((await alice.api.send("DELETE", `${base}/${b.id}`)).status()).toBe(
    204,
  );
  expect((await totals(alice, budget.id)).available_cents).toBe(8000);
  expect((await alice.api.send("DELETE", `${base}/${a.id}`)).status()).toBe(
    204,
  );
  expect((await totals(alice, budget.id)).available_cents).toBe(10000);
  await rejection(await alice.api.send("DELETE", `${base}/${a.id}`), 404);
});
test("recusa excesso, duplicidade e entradas inválidas inclusive erros em texto", async ({
  alice,
}) => {
  const budget = await cycle(alice, 10000);
  const base = `/api/cycles/${budget.id}/envelopes`;
  const a = await envelope(alice, budget.id, "Casa", 10000);
  await rejection(
    await alice.api.send("POST", base, { name: "Casa", allocated_cents: 0 }),
    409,
  );
  await rejection(
    await alice.api.send("POST", base, { name: "Excesso", allocated_cents: 1 }),
    409,
  );
  for (const payload of [
    { name: " ", allocated_cents: 0 },
    { name: "x", allocated_cents: -1 },
    { name: "x", allocated_cents: 1.5 },
    { name: "x", allocated_cents: 0, owner_id: alice.me.user_id },
  ])
    await rejection(await alice.api.send("POST", base, payload), 422);
  for (const payload of [
    {},
    { allocated_cents: null },
    { cycle_id: budget.id },
    { allocated_cents: -1 },
  ])
    await rejection(
      await alice.api.send("PATCH", `${base}/${a.id}`, payload),
      422,
    );
  await rejection(
    await alice.api.send("POST", base, {
      name: "x".repeat(5000),
      allocated_cents: 0,
    }),
    413,
  );
});
test("mutações exigem CSRF e origem válida", async ({ alice }) => {
  const budget = await cycle(alice);
  const base = `/api/cycles/${budget.id}/envelopes`;
  await rejection(
    await alice.api.send(
      "POST",
      base,
      { name: "No CSRF", allocated_cents: 0 },
      { csrf: false },
    ),
    403,
  );
  const original = alice.me.csrf_token;
  alice.api.setCsrf("invalid-test-csrf");
  await rejection(
    await alice.api.send("POST", base, {
      name: "Wrong CSRF",
      allocated_cents: 0,
    }),
    403,
  );
  alice.api.setCsrf(original);
  await rejection(
    await alice.api.send(
      "POST",
      base,
      { name: "Wrong origin", allocated_cents: 0 },
      { origin: "http://untrusted.invalid" },
    ),
    403,
  );
});
