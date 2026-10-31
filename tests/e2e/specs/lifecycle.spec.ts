import { test, expect } from "../fixtures/auth.js";
import { body, rejection, type Cycle } from "../fixtures/api-client.js";
import { cycle, envelope, totals } from "../fixtures/test-data.js";
import { admin } from "../fixtures/environment.js";

test("ativação exige saldo zero e confirma o planejamento; fechamento é definitivo", async ({
  alice,
}) => {
  const budget = await cycle(alice, 100);
  const base = `/api/cycles/${budget.id}`;
  await rejection(await alice.api.send("POST", `${base}/activate`), 409);
  await rejection(await alice.api.send("POST", `${base}/close`), 409);
  expect((await totals(alice, budget.id)).status).toBe("planned");
  const item = await envelope(alice, budget.id, "Total", 100);
  const zero = await envelope(alice, budget.id, "Zero", 0);
  for (const action of ["activate", "activate", "close", "close"]) {
    const value = await body<Cycle>(
      await alice.api.send("POST", `${base}/${action}`),
      200,
      "Cycle",
    );
    expect(value.status).toBe(action === "activate" ? "active" : "closed");
    expect(value.allocated_cents).toBe(100);
    expect(value.available_cents).toBe(0);
    expect((await totals(alice, budget.id)).status).toBe(value.status);
    expect((await alice.api.send("GET", `${base}/envelopes`)).status()).toBe(
      200,
    );
    await rejection(
      await alice.api.send("POST", `${base}/envelopes`, {
        name: "Blocked",
        allocated_cents: 0,
      }),
      409,
    );
    await rejection(
      await alice.api.send("PATCH", `${base}/envelopes/${item.id}`, {
        name: "Blocked",
      }),
      409,
    );
    await rejection(
      await alice.api.send("PATCH", `${base}/envelopes/${item.id}`, {
        allocated_cents: 0,
      }),
      409,
    );
    await rejection(
      await alice.api.send("DELETE", `${base}/envelopes/${zero.id}`),
      409,
    );
    await rejection(
      await alice.api.send("DELETE", `${base}/envelopes/${item.id}`),
      409,
    );
  }
  await rejection(await alice.api.send("POST", `${base}/activate`), 409);
  const listed = await alice.api.send("GET", "/api/cycles?limit=100");
  expect(
    (await listed.json()).find((c: Cycle) => c.id === budget.id).status,
  ).toBe("closed");
});

test("renda zero e somente escrita permitem transições sem conceder leitura HTTP", async ({
  login,
}) => {
  const actor = await login("writer");
  const budget = await cycle(actor, 0);
  for (const action of ["activate", "close"]) {
    const value = await body<Cycle>(
      await actor.api.send("POST", `/api/cycles/${budget.id}/${action}`),
      200,
      "Cycle",
    );
    expect(value.available_cents).toBe(0);
  }
  await rejection(await actor.api.send("GET", `/api/cycles/${budget.id}`), 403);
});

test("transições respeitam proprietário, UUID ausente, CSRF e origem", async ({
  alice,
  login,
}) => {
  const other = await login("bob");
  const budget = await cycle(alice, 0);
  for (const action of ["activate", "close"]) {
    const target = `/api/cycles/${budget.id}/${action}`;
    await rejection(await other.api.send("POST", target), 404);
    await rejection(
      await alice.api.send(
        "POST",
        `/api/cycles/00000000-0000-4000-8000-000000000000/${action}`,
      ),
      404,
    );
    await rejection(
      await alice.api.send("POST", target, undefined, { csrf: false }),
      403,
    );
    await rejection(
      await alice.api.send("POST", target, undefined, {
        origin: "https://other.invalid",
      }),
      403,
    );
  }
  expect((await totals(alice, budget.id)).status).toBe("planned");
});

test("leitura isolada, revogação e bloqueio negam transições na sessão existente", async ({
  login,
}) => {
  const actor = await login("reader", ["budget:read", "budget:write"]);
  const budget = await cycle(actor, 0);
  const base = `/api/cycles/${budget.id}`;
  await admin(["revoke", actor.me.user_id, "budget:write"]);
  for (const action of ["activate", "close"])
    await rejection(await actor.api.send("POST", `${base}/${action}`), 403);
  await admin(["grant", actor.me.user_id, "budget:write"]);
  expect((await actor.api.send("POST", `${base}/activate`)).status()).toBe(200);
  await admin(["revoke", actor.me.user_id, "budget:write"]);
  await rejection(await actor.api.send("POST", `${base}/close`), 403);
  await admin(["grant", actor.me.user_id, "budget:write"]);
  await admin(["disable", actor.me.user_id]);
  await rejection(await actor.api.send("POST", `${base}/close`), 403);
});

test("ativações simultâneas são idempotentes", async ({ alice }) => {
  const budget = await cycle(alice, 100);
  await envelope(alice, budget.id, "Full", 100);
  const results = await Promise.all(
    Array.from({ length: 5 }, () =>
      alice.api.send("POST", `/api/cycles/${budget.id}/activate`),
    ),
  );
  for (const response of results)
    expect((await body<Cycle>(response, 200, "Cycle")).status).toBe("active");
  expect((await totals(alice, budget.id)).available_cents).toBe(0);
});

test("ativação e exclusão concorrentes preservam estado e alocação", async ({
  alice,
}) => {
  for (let i = 0; i < 4; i++) {
    const budget = await cycle(alice, 100);
    const item = await envelope(alice, budget.id, "Full", 100);
    const base = `/api/cycles/${budget.id}`;
    const [activation, removal] = await Promise.all([
      alice.api.send("POST", `${base}/activate`),
      alice.api.send("DELETE", `${base}/envelopes/${item.id}`),
    ]);
    const current = await totals(alice, budget.id);
    if (activation.status() === 200) {
      await rejection(removal, 409);
      expect(current.status).toBe("active");
      expect(current.available_cents).toBe(0);
    } else {
      await rejection(activation, 409);
      expect(removal.status()).toBe(204);
      expect(current.status).toBe("planned");
      expect(current.available_cents).toBe(100);
    }
  }
});
