import { uuidv7 } from "../fixtures/uuid.js";
import { test, expect, type Actor } from "../fixtures/auth.js";
import {
  body,
  rejection,
  type Expense,
  type Envelope,
} from "../fixtures/api-client.js";
import { cycle, envelope, account, totals } from "../fixtures/test-data.js";
import { admin } from "../fixtures/environment.js";
type Card = {
  id: string;
  credit_limit_cents: number;
  outstanding_cents: number;
  available_credit_cents: number;
};
type Invoice = {
  id: string;
  card_id: string;
  status: string;
  purchases_cents: number;
  paid_cents: number;
  outstanding_cents: number;
  period_start: string;
  period_end: string;
  due_on: string;
};
async function card(actor: Actor, limit = 100) {
  return body<Card>(
    await actor.api.send("POST", "/api/cards", {
      name: `Card-${uuidv7()}`,
      opened_on: "0001-01-01",
      credit_limit_cents: limit,
    }),
    201,
    "CreditCard",
  );
}
async function setup(actor: Actor, limit = 100, income = 100, existing?: Card) {
  const c = existing || (await card(actor, limit));
  const b = await cycle(actor, income);
  const e = await envelope(actor, b.id, "Credit", income);
  expect(
    (await actor.api.send("POST", `/api/cycles/${b.id}/activate`)).status(),
  ).toBe(200);
  const due = new Date(b.period_end + "T00:00:00Z");
  due.setUTCDate(due.getUTCDate() + 10);
  const i = await body<Invoice>(
    await actor.api.send("POST", `/api/cards/${c.id}/invoices`, {
      period_start: b.period_start,
      period_end: b.period_end,
      due_on: due.toISOString().slice(0, 10),
    }),
    201,
    "CardInvoice",
  );
  return {
    c,
    b,
    e,
    i,
    base: `/api/cards/${c.id}/invoices/${i.id}`,
    expenses: `/api/cycles/${b.id}/envelopes/${e.id}/expenses`,
  };
}
async function purchase(
  actor: Actor,
  s: Awaited<ReturnType<typeof setup>>,
  amount = 50,
) {
  return body<Expense>(
    await actor.api.send("POST", s.expenses, {
      invoice_id: s.i.id,
      description: "Credit purchase",
      occurred_on: s.b.period_start,
      amount_cents: amount,
    }),
    201,
    "Expense",
  );
}
async function balance(actor: Actor, id: string) {
  return body<{ balance_cents: number }>(
    await actor.api.send("GET", `/api/accounts/${id}`),
    200,
    "Account",
  );
}
async function current(actor: Actor, s: Awaited<ReturnType<typeof setup>>) {
  return body<Invoice>(await actor.api.send("GET", s.base), 200, "CardInvoice");
}
test("compra consome envelope; pagamento parcial e quitação não consomem novamente", async ({
  alice,
}) => {
  const s = await setup(alice);
  const a = await account(alice, 100);
  const x = await purchase(alice, s, 50);
  expect(x.account_id).toBeNull();
  expect(x.invoice_id).toBe(s.i.id);
  expect((await balance(alice, a.id)).balance_cents).toBe(100);
  let c = await body<Card>(
    await alice.api.send("GET", `/api/cards/${s.c.id}`),
    200,
    "CreditCard",
  );
  expect(c.outstanding_cents).toBe(50);
  expect(c.available_credit_cents).toBe(50);
  const pay = {
    request_id: uuidv7(),
    account_id: a.id,
    description: "Bill",
    occurred_on: s.b.period_end,
    amount_cents: 20,
  };
  await rejection(await alice.api.send("POST", s.base + "/payments", pay), 409);
  expect((await alice.api.send("POST", s.base + "/close")).status()).toBe(200);
  expect(
    (await alice.api.send("POST", `/api/cycles/${s.b.id}/close`)).status(),
  ).toBe(200);
  const p1 = await body<any>(
    await alice.api.send("POST", s.base + "/payments", pay),
    201,
    "CardPayment",
  );
  expect((await current(alice, s)).outstanding_cents).toBe(30);
  expect((await balance(alice, a.id)).balance_cents).toBe(80);
  const p2 = await body<any>(
    await alice.api.send("POST", s.base + "/payments", {
      ...pay,
      request_id: uuidv7(),
      amount_cents: 30,
    }),
    201,
    "CardPayment",
  );
  const i = await current(alice, s);
  expect(i.status).toBe("paid");
  expect(i.paid_cents).toBe(50);
  expect(i.outstanding_cents).toBe(0);
  expect((await balance(alice, a.id)).balance_cents).toBe(50);
  c = await body<Card>(
    await alice.api.send("GET", `/api/cards/${s.c.id}`),
    200,
    "CreditCard",
  );
  expect(c.available_credit_cents).toBe(100);
  const envelopes = await (
    await alice.api.send("GET", `/api/cycles/${s.b.id}/envelopes`)
  ).json();
  expect(envelopes[0].spent_cents).toBe(50);
  expect((await totals(alice, s.b.id)).available_cents).toBe(0);
  const statement = await (
    await alice.api.send("GET", `/api/accounts/${a.id}/movements`)
  ).json();
  expect(statement).toHaveLength(2);
  expect(
    statement.every(
      (m: any) => m.kind === "card_payment" && m.expense_id === null,
    ),
  ).toBe(true);
  expect(statement.map((m: any) => m.card_payment_id).sort()).toEqual(
    [p1.id, p2.id].sort(),
  );
  expect(
    (await (await alice.api.send("GET", s.base + "/purchases")).json())[0].id,
  ).toBe(x.id);
  expect(
    await (await alice.api.send("GET", s.base + "/payments")).json(),
  ).toHaveLength(2);
});
test("estorno em fatura aberta recupera limite sem creditar conta; fechada bloqueia novos estornos", async ({
  alice,
}) => {
  const s = await setup(alice);
  const a = await account(alice, 100);
  const first = await purchase(alice, s, 40);
  for (let i = 0; i < 2; i++)
    expect(
      (
        await alice.api.send("POST", `${s.expenses}/${first.id}/reverse`)
      ).status(),
    ).toBe(200);
  expect((await current(alice, s)).purchases_cents).toBe(0);
  expect((await balance(alice, a.id)).balance_cents).toBe(100);
  const second = await purchase(alice, s, 30);
  expect((await alice.api.send("POST", s.base + "/close")).status()).toBe(200);
  expect((await alice.api.send("POST", s.base + "/close")).status()).toBe(200);
  await rejection(
    await alice.api.send("POST", `${s.expenses}/${second.id}/reverse`),
    409,
  );
  await rejection(
    await alice.api.send("POST", s.expenses, {
      invoice_id: s.i.id,
      description: "After close",
      occurred_on: s.b.period_start,
      amount_cents: 1,
    }),
    409,
  );
  expect((await current(alice, s)).outstanding_cents).toBe(30);
});
test("valida cartão, período, sobreposição e origem única da despesa", async ({
  alice,
}) => {
  const s = await setup(alice);
  const a = await account(alice, 100);
  for (const value of [0, -1])
    await rejection(
      await alice.api.send("POST", "/api/cards", {
        name: "Invalid",
        opened_on: "2026-01-01",
        credit_limit_cents: value,
      }),
      422,
    );
  await rejection(
    await alice.api.send("POST", `/api/cards/${s.c.id}/invoices`, {
      period_start: s.i.period_start,
      period_end: s.i.period_end,
      due_on: s.i.due_on,
    }),
    409,
  );
  await rejection(
    await alice.api.send("POST", `/api/cards/${s.c.id}/invoices`, {
      period_start: s.i.period_start,
      period_end: s.i.period_end,
      due_on: s.i.period_start,
    }),
    422,
  );
  const input = {
    description: "Invalid funding",
    occurred_on: s.b.period_start,
    amount_cents: 1,
  };
  for (const funding of [
    {},
    { account_id: a.id, invoice_id: s.i.id },
    { account_id: null, invoice_id: s.i.id },
  ])
    await rejection(
      await alice.api.send("POST", s.expenses, { ...input, ...funding }),
      422,
    );
});
test("limite é compartilhado entre faturas e seguro sob compras simultâneas", async ({
  alice,
}) => {
  const c = await card(alice, 100);
  const a = await setup(alice, 100, 1000, c);
  const b = await setup(alice, 100, 1000, c);
  const results = await Promise.all(
    Array.from({ length: 10 }, (_, i) => {
      const s = i % 2 ? a : b;
      return alice.api.send("POST", s.expenses, {
        invoice_id: s.i.id,
        description: `Race ${i}`,
        occurred_on: s.b.period_start,
        amount_cents: 20,
      });
    }),
  );
  expect(results.filter((r) => r.status() === 201)).toHaveLength(5);
  expect(results.filter((r) => r.status() === 409)).toHaveLength(5);
  const value = await body<Card>(
    await alice.api.send("GET", `/api/cards/${c.id}`),
    200,
    "CreditCard",
  );
  expect(value.outstanding_cents).toBe(100);
  expect(value.available_credit_cents).toBe(0);
});
test("pagamento sem saldo reverte tudo e pagamentos integrais concorrentes debitam uma vez", async ({
  alice,
}) => {
  const s = await setup(alice);
  const a = await account(alice, 0);
  await purchase(alice, s, 50);
  await alice.api.send("POST", s.base + "/close");
  const pay = {
    request_id: uuidv7(),
    account_id: a.id,
    description: "Pay",
    occurred_on: s.b.period_end,
    amount_cents: 50,
  };
  await rejection(await alice.api.send("POST", s.base + "/payments", pay), 409);
  expect((await current(alice, s)).paid_cents).toBe(0);
  expect(
    await (await alice.api.send("GET", s.base + "/payments")).json(),
  ).toHaveLength(0);
  expect(
    (
      await alice.api.send("POST", `/api/accounts/${a.id}/receipts`, {
        description: "Fund",
        occurred_on: s.b.period_end,
        amount_cents: 50,
      })
    ).status(),
  ).toBe(201);
  const result = await Promise.all([
    alice.api.send("POST", s.base + "/payments", pay),
    alice.api.send("POST", s.base + "/payments", pay),
  ]);
  expect(result.map((r) => r.status()).sort()).toEqual([201, 201]);
  expect((await result[0].json()).id).toBe((await result[1].json()).id);
  expect(
    await (await alice.api.send("GET", s.base + "/payments")).json(),
  ).toHaveLength(1);
  expect((await balance(alice, a.id)).balance_cents).toBe(0);
  expect((await current(alice, s)).status).toBe("paid");
});
test("cartões e faturas respeitam proprietário, CSRF, origem e revogação", async ({
  alice,
  login,
}) => {
  const s = await setup(alice);
  await purchase(alice, s, 50);
  const bob = await login("bob");
  const a = await account(bob, 100);
  const pay = {
    request_id: uuidv7(),
    account_id: a.id,
    description: "Forbidden",
    occurred_on: s.b.period_end,
    amount_cents: 1,
  };
  for (const path of [
    `/api/cards/${s.c.id}`,
    s.base,
    s.base + "/payments",
    s.base + "/purchases",
  ])
    await rejection(await bob.api.send("GET", path), 404);
  await rejection(await bob.api.send("POST", s.base + "/close"), 404);
  await rejection(await bob.api.send("POST", s.base + "/payments", pay), 404);
  const other = await setup(bob);
  await rejection(
    await bob.api.send("POST", other.expenses, {
      invoice_id: s.i.id,
      description: "Foreign",
      occurred_on: other.b.period_start,
      amount_cents: 1,
    }),
    404,
  );
  await rejection(
    await alice.api.send("POST", s.base + "/close", undefined, { csrf: false }),
    403,
  );
  await rejection(
    await alice.api.send("POST", s.base + "/close", undefined, {
      origin: "https://other.invalid",
    }),
    403,
  );
  await admin(["revoke", alice.me.user_id, "budget:write"]);
  await rejection(await alice.api.send("POST", s.base + "/close"), 403);
  const writer = await login("writer");
  const own = await setup(writer);
  expect((await writer.api.send("POST", own.base + "/close")).status()).toBe(
    200,
  );
  await rejection(await writer.api.send("GET", own.base), 403);
  const denied = await login("denied");
  await rejection(await denied.api.send("GET", "/api/cards"), 403);
});
test("fechamento concorrente serializa com compra e bloqueia compras posteriores", async ({
  alice,
}) => {
  const s = await setup(alice);
  const [purchase, close] = await Promise.all([
    alice.api.send("POST", s.expenses, {
      invoice_id: s.i.id,
      description: "Race",
      occurred_on: s.b.period_start,
      amount_cents: 50,
    }),
    alice.api.send("POST", s.base + "/close"),
  ]);
  expect(close.status()).toBe(200);
  expect([201, 409]).toContain(purchase.status());
  const i = await current(alice, s);
  expect(i.purchases_cents).toBe(purchase.status() === 201 ? 50 : 0);
  expect(i.status).toBe(purchase.status() === 201 ? "closed" : "paid");
  await rejection(
    await alice.api.send("POST", s.expenses, {
      invoice_id: s.i.id,
      description: "Closed",
      occurred_on: s.b.period_start,
      amount_cents: 1,
    }),
    409,
  );
});

test("identificadores diferentes disputam a dívida e replay com dados alterados é recusado", async ({
  alice,
}) => {
  const s = await setup(alice);
  const a = await account(alice, 100);
  await purchase(alice, s, 50);
  await alice.api.send("POST", s.base + "/close");
  const pay = {
    request_id: uuidv7(),
    account_id: a.id,
    description: "Pay",
    occurred_on: s.b.period_end,
    amount_cents: 50,
  };
  const results = await Promise.all([
    alice.api.send("POST", s.base + "/payments", pay),
    alice.api.send("POST", s.base + "/payments", {
      ...pay,
      request_id: uuidv7(),
    }),
  ]);
  expect(results.map((r) => r.status()).sort()).toEqual([201, 409]);
  const accepted = await results.find((r) => r.status() === 201)!.json();
  await rejection(
    await alice.api.send("POST", s.base + "/payments", {
      ...pay,
      request_id: accepted.id,
      amount_cents: 1,
    }),
    409,
  );
  expect((await balance(alice, a.id)).balance_cents).toBe(50);
  expect((await current(alice, s)).paid_cents).toBe(50);
});
