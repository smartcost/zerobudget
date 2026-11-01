import { uuidv7 } from "../fixtures/uuid.js";
import { test, expect } from "../fixtures/auth.js";
import {
  body,
  rejection,
  type Expense,
  type Envelope,
} from "../fixtures/api-client.js";
import { account, cycle, envelope, totals } from "../fixtures/test-data.js";
import { admin } from "../fixtures/environment.js";
const date = "2100-01-01";
async function cash(actor: any, id: string) {
  return body<{ balance_cents: number }>(
    await actor.api.send("GET", `/api/accounts/${id}`),
    200,
    "Account",
  );
}
test("contas, recebimentos e transferência preservam soma e extratos", async ({
  alice,
}) => {
  const a = await account(alice, 1000);
  const b = await account(alice, 0, "cash");
  const receipt = await body<any>(
    await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, {
      description: "Salary",
      occurred_on: date,
      amount_cents: 500,
    }),
    201,
    "Movement",
  );
  expect(receipt.kind).toBe("receipt");
  const transfer = await body<any>(
    await alice.api.send("POST", "/api/transfers", {
      from_account_id: a.id,
      to_account_id: b.id,
      description: "Wallet",
      occurred_on: date,
      amount_cents: 300,
    }),
    201,
    "Movement",
  );
  expect(transfer.kind).toBe("transfer");
  expect((await cash(alice, a.id)).balance_cents).toBe(1200);
  expect((await cash(alice, b.id)).balance_cents).toBe(300);
  for (const [id, delta] of [
    [a.id, -300],
    [b.id, 300],
  ] as const) {
    const r = await alice.api.send("GET", `/api/accounts/${id}/movements`);
    expect(r.status()).toBe(200);
    const entries = await r.json();
    expect(entries.find((m: any) => m.id === transfer.id).delta_cents).toBe(
      delta,
    );
  }
  expect((await alice.api.send("GET", "/api/accounts")).status()).toBe(200);
});
test("despesa e estorno afetam conta e envelope uma única vez; falta de saldo reverte tudo", async ({
  alice,
}) => {
  const a = await account(alice, 100);
  const b = await cycle(alice, 1000);
  const e = await envelope(alice, b.id, "Food", 1000);
  expect(
    (await alice.api.send("POST", `/api/cycles/${b.id}/activate`)).status(),
  ).toBe(200);
  const path = `/api/cycles/${b.id}/envelopes/${e.id}/expenses`;
  const input = {
    account_id: a.id,
    description: "Food",
    occurred_on: b.period_start,
    amount_cents: 101,
  };
  await rejection(await alice.api.send("POST", path, input), 409);
  expect((await cash(alice, a.id)).balance_cents).toBe(100);
  expect(await (await alice.api.send("GET", path)).json()).toEqual([]);
  let envs = await (
    await alice.api.send("GET", `/api/cycles/${b.id}/envelopes`)
  ).json();
  expect(envs[0].spent_cents).toBe(0);
  const exp = await body<Expense>(
    await alice.api.send("POST", path, { ...input, amount_cents: 100 }),
    201,
    "Expense",
  );
  expect(exp.account_id).toBe(a.id);
  expect((await cash(alice, a.id)).balance_cents).toBe(0);
  for (let i = 0; i < 2; i++)
    expect(
      (await alice.api.send("POST", `${path}/${exp.id}/reverse`)).status(),
    ).toBe(200);
  expect((await cash(alice, a.id)).balance_cents).toBe(100);
  const statement = await (
    await alice.api.send("GET", `/api/accounts/${a.id}/movements`)
  ).json();
  expect(statement.filter((m: any) => m.expense_id === exp.id)).toHaveLength(2);
  expect((await totals(alice, b.id)).income_cents).toBe(1000);
  expect((await totals(alice, b.id)).available_cents).toBe(0);
});
test("valida contas, movimentos, data de abertura e conta obrigatória", async ({
  alice,
}) => {
  const a = await account(alice, 100, "bank", "2100-01-01");
  for (const patch of [
    { name: " " },
    { kind: "credit_card" },
    { opening_balance_cents: -1 },
    { owner_id: alice.me.user_id },
    { opened_on: "bad" },
  ])
    await rejection(
      await alice.api.send("POST", "/api/accounts", {
        name: `Invalid-${uuidv7()}`,
        kind: "bank",
        opened_on: date,
        opening_balance_cents: 0,
        ...patch,
      }),
      422,
    );
  for (const patch of [
    { amount_cents: 0 },
    { amount_cents: -1 },
    { description: "" },
    { occurred_on: "2000-01-01" },
  ])
    await rejection(
      await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, {
        description: "Income",
        occurred_on: date,
        amount_cents: 1,
        ...patch,
      }),
      422,
    );
  await rejection(
    await alice.api.send("POST", "/api/transfers", {
      from_account_id: a.id,
      to_account_id: a.id,
      description: "Same",
      occurred_on: date,
      amount_cents: 1,
    }),
    422,
  );
  const b = await cycle(alice, 1);
  const e = await envelope(alice, b.id, "Full", 1);
  await alice.api.send("POST", `/api/cycles/${b.id}/activate`);
  await rejection(
    await alice.api.send(
      "POST",
      `/api/cycles/${b.id}/envelopes/${e.id}/expenses`,
      {
        description: "No account",
        occurred_on: b.period_start,
        amount_cents: 1,
      },
    ),
    422,
  );
});
test("isolamento, CSRF, origem, revogação e escrita sem leitura nas contas", async ({
  alice,
  login,
}) => {
  const a = await account(alice, 100);
  const bob = await login("bob");
  const b = await account(bob, 100);
  await rejection(await bob.api.send("GET", `/api/accounts/${a.id}`), 404);
  await rejection(
    await bob.api.send("GET", `/api/accounts/${a.id}/movements`),
    404,
  );
  const input = { description: "Income", occurred_on: date, amount_cents: 1 };
  await rejection(
    await bob.api.send("POST", `/api/accounts/${a.id}/receipts`, input),
    404,
  );
  await rejection(
    await bob.api.send("POST", "/api/transfers", {
      ...input,
      from_account_id: b.id,
      to_account_id: a.id,
    }),
    404,
  );
  await rejection(
    await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, input, {
      csrf: false,
    }),
    403,
  );
  await rejection(
    await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, input, {
      origin: "https://other.invalid",
    }),
    403,
  );
  await admin(["revoke", alice.me.user_id, "budget:write"]);
  await rejection(
    await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, input),
    403,
  );
  const writer = await login("writer");
  const own = await account(writer, 0);
  expect(
    (
      await writer.api.send("POST", `/api/accounts/${own.id}/receipts`, input)
    ).status(),
  ).toBe(201);
  await rejection(await writer.api.send("GET", `/api/accounts/${own.id}`), 403);
});
test("transferências concorrentes não gastam saldo duas vezes", async ({
  alice,
}) => {
  const a = await account(alice, 100);
  const b = await account(alice, 0, "cash");
  const results = await Promise.all(
    Array.from({ length: 10 }, () =>
      alice.api.send("POST", "/api/transfers", {
        from_account_id: a.id,
        to_account_id: b.id,
        description: "Race",
        occurred_on: date,
        amount_cents: 20,
      }),
    ),
  );
  expect(results.filter((r) => r.status() === 201)).toHaveLength(5);
  expect(results.filter((r) => r.status() === 409)).toHaveLength(5);
  expect((await cash(alice, a.id)).balance_cents).toBe(0);
  expect((await cash(alice, b.id)).balance_cents).toBe(100);
});
test("duas despesas em ciclos diferentes disputam o mesmo saldo bancário", async ({
  alice,
}) => {
  const a = await account(alice, 100);
  const targets = [];
  for (let i = 0; i < 2; i++) {
    const b = await cycle(alice, 100);
    const e = await envelope(alice, b.id, "Full", 100);
    await alice.api.send("POST", `/api/cycles/${b.id}/activate`);
    targets.push({ b, e });
  }
  const result = await Promise.all(
    targets.map(({ b, e }) =>
      alice.api.send("POST", `/api/cycles/${b.id}/envelopes/${e.id}/expenses`, {
        account_id: a.id,
        description: "Race",
        occurred_on: b.period_start,
        amount_cents: 100,
      }),
    ),
  );
  expect(result.map((r) => r.status()).sort()).toEqual([201, 409]);
  expect((await cash(alice, a.id)).balance_cents).toBe(0);
});
