import { test, expect, type Actor } from "../fixtures/auth.js";
import {
  body,
  rejection,
  type Expense,
  type Envelope,
} from "../fixtures/api-client.js";
import { cycle, envelope, totals, account } from "../fixtures/test-data.js";
import { admin } from "../fixtures/environment.js";
async function setup(actor: Actor, amount = 10000, activate = true) {
  const budget = await cycle(actor, amount);
  const funding = await account(actor, amount);
  const item = await envelope(actor, budget.id, "Expenses", amount);
  const root = `/api/cycles/${budget.id}`;
  if (activate)
    expect((await actor.api.send("POST", `${root}/activate`)).status()).toBe(
      200,
    );
  return {
    budget,
    funding,
    item,
    root,
    path: `${root}/envelopes/${item.id}/expenses`,
  };
}
async function balance(actor: Actor, cycleId: string, envelopeId: string) {
  const response = await actor.api.send(
    "GET",
    `/api/cycles/${cycleId}/envelopes`,
  );
  expect(response.status()).toBe(200);
  const values = await response.json();
  const found = values.find((e: Envelope) => e.id === envelopeId);
  expect(found).toBeTruthy();
  return found as Envelope;
}
test("despesa, saldo e estorno preservam o histórico e a alocação", async ({
  alice,
}) => {
  const { budget, item, path, funding } = await setup(alice);
  const input = {
    description: " Food ",
    occurred_on: budget.period_start,
    account_id: funding.id,
    amount_cents: 1200,
  };
  const expense = await body<Expense>(
    await alice.api.send("POST", path, input),
    201,
    "Expense",
  );
  expect(expense.description).toBe("Food");
  expect(expense.status).toBe("posted");
  let current = await balance(alice, budget.id, item.id);
  expect(current.spent_cents).toBe(1200);
  expect(current.remaining_cents).toBe(8800);
  expect((await totals(alice, budget.id)).available_cents).toBe(0);
  for (let i = 0; i < 2; i++) {
    const reversed = await body<Expense>(
      await alice.api.send("POST", `${path}/${expense.id}/reverse`),
      200,
      "Expense",
    );
    expect(reversed.status).toBe("reversed");
    expect(reversed.amount_cents).toBe(1200);
    expect(reversed.description).toBe("Food");
  }
  current = await balance(alice, budget.id, item.id);
  expect(current.spent_cents).toBe(0);
  expect(current.remaining_cents).toBe(10000);
  const list = await alice.api.send("GET", path);
  expect(list.status()).toBe(200);
  expect(await list.json()).toEqual([{ ...expense, status: "reversed" }]);
});
test("valores, descrição, data, período e campos proibidos são validados", async ({
  alice,
}) => {
  const { budget, path, funding } = await setup(alice);
  const valid = {
    description: "Food",
    occurred_on: budget.period_start,
    account_id: funding.id,
    amount_cents: 1,
  };
  for (const patch of [
    { amount_cents: 0 },
    { amount_cents: -1 },
    { amount_cents: 1.5 },
    { description: " " },
    { description: "a\nb" },
    { description: "a".repeat(201) },
    { occurred_on: "2026-02-30" },
    { occurred_on: budget.period_end },
    { occurred_on: "2000-01-01" },
    { owner_id: alice.me.user_id },
    { envelope_id: "forged" },
  ])
    await rejection(
      await alice.api.send("POST", path, { ...valid, ...patch }),
      422,
    );
  await rejection(await alice.api.send("GET", `${path}?limit=101`), 422);
  await body<Expense>(
    await alice.api.send("POST", path, valid),
    201,
    "Expense",
  );
});
test("recusa excesso e só registra ou estorna em ciclo ativo; fechado preserva consultas", async ({
  alice,
}) => {
  const { budget, item, root, path, funding } = await setup(alice, 100, false);
  const input = {
    description: "All",
    occurred_on: budget.period_start,
    account_id: funding.id,
    amount_cents: 100,
  };
  await rejection(await alice.api.send("POST", path, input), 409);
  expect((await alice.api.send("POST", `${root}/activate`)).status()).toBe(200);
  const expense = await body<Expense>(
    await alice.api.send("POST", path, input),
    201,
    "Expense",
  );
  await rejection(
    await alice.api.send("POST", path, { ...input, amount_cents: 1 }),
    409,
  );
  expect((await alice.api.send("POST", `${root}/close`)).status()).toBe(200);
  await rejection(await alice.api.send("POST", path, input), 409);
  await rejection(
    await alice.api.send("POST", `${path}/${expense.id}/reverse`),
    409,
  );
  expect((await alice.api.send("GET", path)).status()).toBe(200);
  expect((await balance(alice, budget.id, item.id)).spent_cents).toBe(100);
});
test("isolamento por proprietário e vínculo entre ciclo, envelope e despesa", async ({
  alice,
  login,
}) => {
  const { budget, path, funding } = await setup(alice);
  const input = {
    description: "Private",
    occurred_on: budget.period_start,
    account_id: funding.id,
    amount_cents: 1,
  };
  const expense = await body<Expense>(
    await alice.api.send("POST", path, input),
    201,
    "Expense",
  );
  const bob = await login("bob");
  await rejection(await bob.api.send("GET", path), 404);
  await rejection(await bob.api.send("POST", path, input), 404);
  await rejection(
    await bob.api.send("POST", `${path}/${expense.id}/reverse`),
    404,
  );
  const other = await setup(alice);
  await rejection(
    await alice.api.send("POST", `${other.path}/${expense.id}/reverse`),
    404,
  );
  await rejection(
    await alice.api.send(
      "GET",
      `${other.root}${path.slice(path.indexOf("/envelopes"))}`,
    ),
    404,
  );
  await rejection(
    await alice.api.send(
      "POST",
      `${path}/00000000-0000-4000-8000-000000000000/reverse`,
    ),
    404,
  );
});
test("sessão, CSRF, origem e permissões atuais protegem despesas e estornos", async ({
  alice,
  login,
  request,
}) => {
  const { budget, path, funding } = await setup(alice);
  const input = {
    description: "Protected",
    occurred_on: budget.period_start,
    account_id: funding.id,
    amount_cents: 1,
  };
  const expense = await body<Expense>(
    await alice.api.send("POST", path, input),
    201,
    "Expense",
  );
  for (const target of [path, `${path}/${expense.id}/reverse`]) {
    await rejection(await request.post(target, { data: input }), 401);
    await rejection(
      await alice.api.send("POST", target, input, { csrf: false }),
      403,
    );
    await rejection(
      await alice.api.send("POST", target, input, {
        origin: "https://other.invalid",
      }),
      403,
    );
  }
  await admin(["revoke", alice.me.user_id, "budget:write"]);
  await rejection(await alice.api.send("POST", path, input), 403);
  await rejection(
    await alice.api.send("POST", `${path}/${expense.id}/reverse`),
    403,
  );
  await admin(["grant", alice.me.user_id, "budget:write"]);
  await admin(["disable", alice.me.user_id]);
  await rejection(await alice.api.send("POST", path, input), 403);
  const denied = await login("denied");
  await rejection(await denied.api.send("GET", path), 403);
});
test("somente escrita registra e estorna sem conceder GET", async ({
  login,
}) => {
  const actor = await login("writer");
  const { budget, path, funding } = await setup(actor);
  const expense = await body<Expense>(
    await actor.api.send("POST", path, {
      description: "Write",
      occurred_on: budget.period_start,
      account_id: funding.id,
      amount_cents: 100,
    }),
    201,
    "Expense",
  );
  expect(
    (await actor.api.send("POST", `${path}/${expense.id}/reverse`)).status(),
  ).toBe(200);
  await rejection(await actor.api.send("GET", path), 403);
});
test("despesas concorrentes respeitam o saldo e estornos simultâneos creditam uma vez", async ({
  alice,
}) => {
  const { budget, item, path, funding } = await setup(alice, 100);
  const results = await Promise.all(
    Array.from({ length: 10 }, (_, i) =>
      alice.api.send("POST", path, {
        description: `Race ${i}`,
        occurred_on: budget.period_start,
        account_id: funding.id,
        amount_cents: 20,
      }),
    ),
  );
  expect(results.filter((r) => r.status() === 201)).toHaveLength(5);
  expect(results.filter((r) => r.status() === 409)).toHaveLength(5);
  let current = await balance(alice, budget.id, item.id);
  expect(current.spent_cents).toBe(100);
  expect(current.remaining_cents).toBe(0);
  const expense = await body<Expense>(
    results.find((r) => r.status() === 201)!,
    201,
    "Expense",
  );
  const reversed = await Promise.all(
    Array.from({ length: 5 }, () =>
      alice.api.send("POST", `${path}/${expense.id}/reverse`),
    ),
  );
  for (const r of reversed)
    expect((await body<Expense>(r, 200, "Expense")).status).toBe("reversed");
  current = await balance(alice, budget.id, item.id);
  expect(current.spent_cents).toBe(80);
  expect(current.remaining_cents).toBe(20);
});
test("fechamento concorrente serializa com despesas", async ({ alice }) => {
  for (let i = 0; i < 3; i++) {
    const { budget, item, root, path, funding } = await setup(alice, 100);
    const [write, close] = await Promise.all([
      alice.api.send("POST", path, {
        description: "Race close",
        occurred_on: budget.period_start,
        account_id: funding.id,
        amount_cents: 100,
      }),
      alice.api.send("POST", `${root}/close`),
    ]);
    expect(close.status()).toBe(200);
    expect([201, 409]).toContain(write.status());
    expect((await totals(alice, budget.id)).status).toBe("closed");
    expect((await balance(alice, budget.id, item.id)).spent_cents).toBe(
      write.status() === 201 ? 100 : 0,
    );
    await rejection(
      await alice.api.send("POST", path, {
        description: "After close",
        occurred_on: budget.period_start,
        account_id: funding.id,
        amount_cents: 1,
      }),
      409,
    );
  }
});
