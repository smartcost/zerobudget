import { test, expect, type Me } from '../fixtures/auth.js';
import { ApiClient, body, rejection, type Expense } from '../fixtures/api-client.js';
import { register, signIn, password } from '../fixtures/registration.js';
import { state, admin } from '../fixtures/environment.js';
import { account, cycle, envelope } from '../fixtures/test-data.js';
type Status = {
    status: string;
    completed_at: number | null;
    permissions: string[];
    next_step: string;
    setup: {
        has_accounts: boolean;
        has_cycles: boolean;
        has_active_cycle: boolean;
        has_expenses: boolean;
        selected_cycle: {
            id: string;
            status: string;
            has_envelopes: boolean;
            has_unallocated_income: boolean;
        } | null;
    } | null;
};
async function current(api: ApiClient) { return body<Status>(await api.send('GET', '/api/onboarding'), 200, 'Onboarding'); }
test('cadastro até a primeira despesa sem concessão administrativa de permissões', async ({ browser }) => {
    const r = await register(browser, `onboarding-${crypto.randomUUID()}@e2e.invalid`);
    const api = new ApiClient(r.context.request, r.me.csrf_token);
    const actor = { api };
    try {
        const before = await current(api);
        expect(before.status).toBe('pending');
        expect(before.next_step).toBe('complete_onboarding');
        expect(before.permissions).toEqual([]);
        expect(before.setup).toBeNull();
        await rejection(await api.send('GET', '/api/cycles'), 403);
        await rejection(await api.send('POST', '/api/onboarding', {}, { csrf: false }), 403);
        await rejection(await api.send('POST', '/api/onboarding', { owner_id: crypto.randomUUID() }), 422);
        const completed = await body<Status>(await api.send('POST', '/api/onboarding', {}), 200, 'Onboarding');
        expect(completed.status).toBe('completed');
        expect(completed.permissions).toEqual(['budget:read', 'budget:write']);
        expect(completed.next_step).toBe('create_account');
        const copies = await Promise.all([api.send('POST', '/api/onboarding', {}), api.send('POST', '/api/onboarding', {})]);
        for (const response of copies)
            expect(await body<Status>(response, 200, 'Onboarding')).toEqual(completed);
        const a = await account(actor, 10000);
        expect((await current(api)).next_step).toBe('create_cycle');
        const c = await cycle(actor, 10000);
        let status = await current(api);
        expect(status.next_step).toBe('allocate_income');
        expect(status.setup?.selected_cycle?.id).toBe(c.id);
        const e = await envelope(actor, c.id, 'Alimentação', 10000);
        expect((await current(api)).next_step).toBe('activate_cycle');
        await body(await api.send('POST', `/api/cycles/${c.id}/activate`), 200, 'Cycle');
        expect((await current(api)).next_step).toBe('record_expense');
        const x = await body<Expense>(await api.send('POST', `/api/cycles/${c.id}/envelopes/${e.id}/expenses`, { request_id: crypto.randomUUID(), account_id: a.id, description: 'Primeira despesa', occurred_on: c.period_start, amount_cents: 2500 }), 201, 'Expense');
        status = await current(api);
        expect(status.next_step).toBe('ready');
        expect(status.setup?.has_expenses).toBe(true);
        expect(status.setup?.has_active_cycle).toBe(true);
        const balance = await body<{
            balance_cents: number;
        }>(await api.send('GET', `/api/accounts/${a.id}`), 200, 'Account');
        expect(balance.balance_cents).toBe(7500);
        await body(await api.send('POST', `/api/cycles/${c.id}/envelopes/${e.id}/expenses/${x.id}/reverse`), 200, 'Expense');
        expect((await current(api)).next_step).toBe('ready');
    }
    finally {
        await r.context.close();
    }
});
test('revogação e bloqueio sobrevivem ao onboarding repetido e a novos logins', async ({ browser }) => {
    const email = `revoked-${crypto.randomUUID()}@e2e.invalid`;
    const r = await register(browser, email);
    const api = new ApiClient(r.context.request, r.me.csrf_token);
    let other: Awaited<ReturnType<typeof signIn>> | undefined;
    try {
        const completed = await body<Status>(await api.send('POST', '/api/onboarding', {}), 200, 'Onboarding');
        await admin(['revoke', r.me.user_id, 'budget:write']);
        let result = await body<Status>(await api.send('POST', '/api/onboarding', {}), 200, 'Onboarding');
        expect(result.permissions).toEqual(['budget:read']);
        expect(result.next_step).toBe('contact_operator');
        expect(result.completed_at).toBe(completed.completed_at);
        other = await signIn(browser, email, password);
        await other.page.waitForURL(state().origin + '/auth/session');
        const me = await body<Me>(await other.context.request.get('/api/auth/me'), 200, 'Me');
        expect(me.permissions).toEqual(['budget:read']);
        const nextApi = new ApiClient(other.context.request, me.csrf_token);
        expect((await current(nextApi)).permissions).toEqual(['budget:read']);
        await admin(['revoke', r.me.user_id, 'budget:read']);
        result = await body<Status>(await nextApi.send('POST', '/api/onboarding', {}), 200, 'Onboarding');
        expect(result.permissions).toEqual([]);
        expect(result.setup).toBeNull();
        await admin(['disable', r.me.user_id]);
        await rejection(await nextApi.send('POST', '/api/onboarding', {}), 403);
        await admin(['enable', r.me.user_id]);
        expect((await current(nextApi)).permissions).toEqual([]);
        expect((await body<Status>(await nextApi.send('POST', '/api/onboarding', {}), 200, 'Onboarding')).permissions).toEqual([]);
    }
    finally {
        await admin(['enable', r.me.user_id]);
        await r.context.close();
        await other?.context.close();
    }
});
test('decisão administrativa anterior impede auto-habilitação e progresso não vaza entre usuários', async ({ browser, alice }) => {
    const r = await register(browser, `managed-${crypto.randomUUID()}@e2e.invalid`);
    const api = new ApiClient(r.context.request, r.me.csrf_token);
    try {
        await account(alice, 100);
        await cycle(alice, 100);
        expect((await current(api)).setup).toBeNull();
        await admin(['grant', r.me.user_id, 'budget:read']);
        await admin(['revoke', r.me.user_id, 'budget:read']);
        expect((await current(api)).status).toBe('managed');
        await rejection(await api.send('POST', '/api/onboarding', {}), 409);
        expect((await current(api)).permissions).toEqual([]);
        await admin(['grant', r.me.user_id, 'budget:read']);
        const s = await current(api);
        expect(s.setup).toEqual({ has_accounts: false, has_cycles: false, has_active_cycle: false, has_expenses: false, selected_cycle: null });
        expect(s.next_step).toBe('contact_operator');
    }
    finally {
        await r.context.close();
    }
});
test('consulta exige sessão e ciclo de renda zero pode ser ativado sem envelopes', async ({ browser, request }) => {
    await rejection(await request.get('/api/onboarding'), 401);
    const r = await register(browser, `zero-${crypto.randomUUID()}@e2e.invalid`);
    const api = new ApiClient(r.context.request, r.me.csrf_token);
    const actor = { api };
    try {
        await body(await api.send('POST', '/api/onboarding', {}), 200, 'Onboarding');
        await account(actor, 0);
        const c = await cycle(actor, 0);
        expect((await current(api)).next_step).toBe('activate_cycle');
        await api.send('POST', `/api/cycles/${c.id}/activate`);
        expect((await current(api)).next_step).toBe('record_expense');
        await api.send('POST', `/api/cycles/${c.id}/close`);
        expect((await current(api)).next_step).toBe('create_cycle');
    }
    finally {
        await r.context.close();
    }
});
