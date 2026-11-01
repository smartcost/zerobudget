import { uuidv7 } from "../fixtures/uuid.js";
import { test, expect, type Actor } from '../fixtures/auth.js';
import { body, rejection, type Cycle, type Envelope, type Expense } from '../fixtures/api-client.js';
import { cycle, envelope, account } from '../fixtures/test-data.js';
type Card = {
    id: string;
    outstanding_cents: number;
    credit_balance_cents: number;
};
type Invoice = {
    id: string;
    outstanding_cents: number;
    credit_balance_cents: number;
    opening_debt_cents: number;
    paid_cents: number;
};
async function card(a: Actor, limit = 1000) { return body<Card>(await a.api.send('POST', '/api/cards', { name: uuidv7(), opened_on: '0001-01-01', credit_limit_cents: limit }), 201, 'CreditCard'); }
async function invoice(a: Actor, c: Card, start: string, end: string) { return body<Invoice>(await a.api.send('POST', `/api/cards/${c.id}/invoices`, { period_start: start, period_end: end, due_on: end }), 201, 'CardInvoice'); }
async function balance(a: Actor, id: string) { return (await body<{
    balance_cents: number;
}>(await a.api.send('GET', `/api/accounts/${id}`), 200, 'Account')).balance_cents; }
const key = () => uuidv7();
test('realocação preserva gasto, serializa concorrência e replay após fechamento', async ({ alice }) => {
    const c = await cycle(alice, 100), a = await account(alice, 100), e = await envelope(alice, c.id, 'Source', 70), f = await envelope(alice, c.id, 'Target', 30);
    await body(await alice.api.send('POST', `/api/cycles/${c.id}/activate`), 200, 'Cycle');
    const expense = { request_id: key(), account_id: a.id, description: 'Gasto', occurred_on: c.period_start, amount_cents: 50 };
    const url = `/api/cycles/${c.id}/envelopes/${e.id}/expenses`;
    const x = await body<Expense>(await alice.api.send('POST', url, expense), 201, 'Expense');
    const move = { request_id: key(), from_envelope_id: e.id, to_envelope_id: f.id, amount_cents: 20 };
    const route = `/api/cycles/${c.id}/reallocations`;
    const replies = await Promise.all([alice.api.send('POST', route, move), alice.api.send('POST', route, move)]);
    for (const r of replies)
        await body(r, 201, 'Reallocation');
    await rejection(await alice.api.send('POST', route, { ...move, amount_cents: 21 }), 409);
    await rejection(await alice.api.send('POST', route, { ...move, request_id: key(), amount_cents: 1 }), 409);
    const all = await (await alice.api.send('GET', `/api/cycles/${c.id}/envelopes`)).json() as Envelope[];
    expect(all.find(v => v.id === e.id)?.remaining_cents).toBe(0);
    expect(all.find(v => v.id === f.id)?.allocated_cents).toBe(50);
    await body(await alice.api.send('POST', `/api/cycles/${c.id}/close`), 200, 'Cycle');
    expect((await body<Expense>(await alice.api.send('POST', url, expense), 201, 'Expense')).id).toBe(x.id);
    await body(await alice.api.send('POST', route, move), 201, 'Reallocation');
    expect(await balance(alice, a.id)).toBe(50);
});
test('recebimentos e transferências: retries, compensações e saldo insuficiente atômico', async ({ alice }) => {
    const a = await account(alice, 0), b = await account(alice, 0);
    const receipt = { request_id: key(), description: 'Recebimento', occurred_on: '2026-10-01', amount_cents: 100 };
    const route = `/api/accounts/${a.id}/receipts`;
    const r = await body<{
        id: string;
    }>(await alice.api.send('POST', route, receipt), 201, 'Movement');
    await body(await alice.api.send('POST', route, receipt), 201, 'Movement');
    await rejection(await alice.api.send('POST', route, { ...receipt, amount_cents: 101 }), 409);
    const transfer = { request_id: key(), from_account_id: a.id, to_account_id: b.id, description: 'Transferência', occurred_on: '2026-10-02', amount_cents: 60 };
    const t = await body<{
        id: string;
    }>(await alice.api.send('POST', '/api/transfers', transfer), 201, 'Movement');
    await body(await alice.api.send('POST', '/api/transfers', transfer), 201, 'Movement');
    expect(await balance(alice, b.id)).toBe(60);
    const correction = { request_id: key(), description: 'Correção', occurred_on: '2026-10-03' };
    await rejection(await alice.api.send('POST', `/api/movements/${r.id}/reverse`, correction), 409);
    expect(await balance(alice, a.id)).toBe(40);
    await body(await alice.api.send('POST', `/api/movements/${t.id}/reverse`, correction), 201, 'Correction');
    await body(await alice.api.send('POST', `/api/movements/${t.id}/reverse`, correction), 201, 'Correction');
    await rejection(await alice.api.send('POST', `/api/movements/${t.id}/reverse`, { ...correction, request_id: key() }), 409);
    await body(await alice.api.send('POST', `/api/movements/${r.id}/reverse`, { ...correction, request_id: key() }), 201, 'Correction');
    expect(await balance(alice, a.id)).toBe(0);
    expect(await balance(alice, b.id)).toBe(0);
    const history = await (await alice.api.send('GET', `/api/accounts/${a.id}/movements`)).json();
    expect(history.filter((m: {
        kind: string;
    }) => m.kind === 'correction')).toHaveLength(2);
});
test('dívida inicial, pagamento corrigido e crédito não viram renda nem despesa', async ({ alice }) => {
    const c = await card(alice), i = await invoice(alice, c, '2028-01-01', '2028-02-01'), a = await account(alice, 1000);
    const base = `/api/cards/${c.id}/invoices/${i.id}`;
    const opening = { request_id: key(), invoice_id: i.id, description: 'Dívida inicial', occurred_on: '2028-01-01', amount_cents: 100 };
    const d = await body<{
        id: string;
    }>(await alice.api.send('POST', `/api/cards/${c.id}/opening-debts`, opening), 201, 'CreditEntry');
    await body(await alice.api.send('POST', `/api/cards/${c.id}/opening-debts`, opening), 201, 'CreditEntry');
    await rejection(await alice.api.send('POST', `/api/cards/${c.id}/opening-debts`, { ...opening, request_id: key() }), 409);
    await body(await alice.api.send('POST', base + '/close'), 200, 'CardInvoice');
    const pay = { request_id: key(), account_id: a.id, description: 'Pagamento errado', occurred_on: '2028-02-01', amount_cents: 100 };
    await body(await alice.api.send('POST', base + '/payments', pay), 201, 'CardPayment');
    expect(await balance(alice, a.id)).toBe(900);
    const statement = await (await alice.api.send('GET', `/api/accounts/${a.id}/movements`)).json();
    const movement = statement.find((m: {
        card_payment_id: string;
    }) => m.card_payment_id === pay.request_id);
    await body(await alice.api.send('POST', `/api/movements/${movement.id}/reverse`, { request_id: key(), description: 'Compensação', occurred_on: '2028-02-02' }), 201, 'Correction');
    expect(await balance(alice, a.id)).toBe(1000);
    expect((await body<Invoice>(await alice.api.send('GET', base), 200, 'CardInvoice')).outstanding_cents).toBe(100);
    expect((await body<{
        status: string;
    }>(await alice.api.send('POST', base + '/payments', pay), 201, 'CardPayment')).status).toBe('reversed');
    const j = await invoice(alice, c, '2028-02-01', '2028-03-01');
    const credit = { request_id: key(), invoice_id: j.id, opening_debt_id: d.id, description: 'Corrigir dívida inicial', occurred_on: '2028-02-02' };
    await body(await alice.api.send('POST', `/api/cards/${c.id}/credits`, credit), 201, 'CreditEntry');
    await body(await alice.api.send('POST', `/api/cards/${c.id}/credits`, credit), 201, 'CreditEntry');
    expect((await body<Invoice>(await alice.api.send('GET', `/api/cards/${c.id}/invoices/${j.id}`), 200, 'CardInvoice')).credit_balance_cents).toBe(100);
    expect(await balance(alice, a.id)).toBe(1000);
});
test('devolução de compra fechada gera crédito futuro e mantém orçamento histórico', async ({ alice }) => {
    const b = await cycle(alice, 100), e = await envelope(alice, b.id, 'Compra', 100), c = await card(alice), i = await invoice(alice, c, b.period_start, b.period_end);
    await alice.api.send('POST', `/api/cycles/${b.id}/activate`);
    const x = await body<Expense>(await alice.api.send('POST', `/api/cycles/${b.id}/envelopes/${e.id}/expenses`, { invoice_id: i.id, description: 'Compra', occurred_on: b.period_start, amount_cents: 100 }), 201, 'Expense');
    await alice.api.send('POST', `/api/cards/${c.id}/invoices/${i.id}/close`);
    await alice.api.send('POST', `/api/cycles/${b.id}/close`);
    const end = new Date(b.period_end + 'T00:00:00Z');
    end.setUTCDate(end.getUTCDate() + 1);
    const j = await invoice(alice, c, b.period_end, end.toISOString().slice(0, 10));
    const refund = { request_id: key(), invoice_id: j.id, expense_id: x.id, description: 'Devolução', occurred_on: b.period_end };
    await body(await alice.api.send('POST', `/api/cards/${c.id}/credits`, refund), 201, 'CreditEntry');
    await rejection(await alice.api.send('POST', `/api/cards/${c.id}/credits`, { ...refund, request_id: key() }), 409);
    const summary = await body<{
        spent_cents: number;
    }>(await alice.api.send('GET', `/api/cycles/${b.id}/summary`), 200, 'CycleSummary');
    expect(summary.spent_cents).toBe(100);
    expect((await body<Card>(await alice.api.send('GET', `/api/cards/${c.id}`), 200, 'CreditCard')).outstanding_cents).toBe(0);
});
test('próximo ciclo transporta saldo uma vez, sem recebimento bancário', async ({ alice }) => {
    const b = await cycle(alice, 100), e = await envelope(alice, b.id, 'Saldo', 100), a = await account(alice, 100);
    await alice.api.send('POST', `/api/cycles/${b.id}/activate`);
    await body(await alice.api.send('POST', `/api/cycles/${b.id}/envelopes/${e.id}/expenses`, { account_id: a.id, description: 'Gasto', occurred_on: b.period_start, amount_cents: 30 }), 201, 'Expense');
    await alice.api.send('POST', `/api/cycles/${b.id}/close`);
    // Reserve a distinct future interval without colliding with fixture counter.
    const offset = await cycle(alice, 0);
    const input = { request_id: key(), period_start: offset.period_end, period_end: '2199-01-01', new_income_cents: 50, mode: 'carry_remaining' };
    const url = `/api/cycles/${b.id}/next`;
    const n = await body<Cycle & {
        carried_cents: number;
    }>(await alice.api.send('POST', url, input), 201, 'Cycle');
    expect(n.income_cents).toBe(120);
    expect(n.carried_cents).toBe(70);
    expect(n.allocated_cents).toBe(70);
    expect(n.status).toBe('planned');
    expect((await body<Cycle>(await alice.api.send('POST', url, input), 201, 'Cycle')).id).toBe(n.id);
    await rejection(await alice.api.send('POST', url, { ...input, request_id: key() }), 409);
    expect(await balance(alice, a.id)).toBe(70);
    const summary = await body<{
        received_cents: number;
        projected_income_gap_cents: number;
    }>(await alice.api.send('GET', `/api/cycles/${n.id}/summary`), 200, 'CycleSummary');
    expect(summary.received_cents).toBe(0);
    expect(summary.projected_income_gap_cents).toBe(50);
});
test('cópia exige renda suficiente e resumo distingue previsão de recebimento', async ({ alice }) => {
    const b = await cycle(alice, 100);
    await envelope(alice, b.id, 'Plano', 100);
    await alice.api.send('POST', `/api/cycles/${b.id}/activate`);
    const a = await account(alice, 0);
    await body(await alice.api.send('POST', `/api/accounts/${a.id}/receipts`, { description: 'Realizado', occurred_on: b.period_start, amount_cents: 40 }), 201, 'Movement');
    let s = await body<{
        received_cents: number;
        projected_income_gap_cents: number;
    }>(await alice.api.send('GET', `/api/cycles/${b.id}/summary`), 200, 'CycleSummary');
    expect(s.received_cents).toBe(40);
    expect(s.projected_income_gap_cents).toBe(60);
    await alice.api.send('POST', `/api/cycles/${b.id}/close`);
    const reserve = await cycle(alice, 0);
    const input = { request_id: key(), period_start: reserve.period_end, period_end: '2198-01-01', new_income_cents: 99, mode: 'copy' };
    await rejection(await alice.api.send('POST', `/api/cycles/${b.id}/next`, input), 409);
    const n = await body<Cycle>(await alice.api.send('POST', `/api/cycles/${b.id}/next`, { ...input, new_income_cents: 100 }), 201, 'Cycle');
    expect(n.allocated_cents).toBe(100);
});
test('novas operações respeitam proprietário, leitura, CSRF e readiness', async ({ alice, login }) => {
    const b = await cycle(alice, 0), c = await card(alice);
    const bob = await login('bob'), reader = await login('reader');
    await rejection(await bob.api.send('GET', `/api/cycles/${b.id}/summary`), 404);
    await rejection(await bob.api.send('GET', `/api/cards/${c.id}/credits`), 404);
    await rejection(await reader.api.send('POST', `/api/cards/${c.id}/opening-debts`, { request_id: key(), invoice_id: key(), description: 'Sem direito', occurred_on: '2028-01-01', amount_cents: 1 }), 403);
    await rejection(await alice.api.send('POST', `/api/cycles/${b.id}/reallocations`, { request_id: key(), from_envelope_id: key(), to_envelope_id: key(), amount_cents: 1 }, { csrf: false }), 403);
    await body(await alice.api.send('GET', '/api/overview'), 200, 'Overview');
    expect((await alice.api.send('GET', '/ready')).status()).toBe(204);
});
