import { uuidv7 } from "../fixtures/uuid.js";
import { test, expect, type Me } from '../fixtures/auth.js';
import { body, rejection } from '../fixtures/api-client.js';
import { state } from '../fixtures/environment.js';
import { password, emailLink, register, signIn } from '../fixtures/registration.js';
import type { BrowserContext } from '@playwright/test';
test('signup exige confirmação de e-mail e provisiona usuário sem permissões implícitas', async ({ browser }) => {
    const r = await register(browser, `signup-${uuidv7()}@e2e.invalid`);
    try {
        expect(r.me.permissions).toEqual([]);
        await rejection(await r.context.request.get('/api/cycles'), 403);
        expect(r.me.issuer).toBe(state().issuer);
    }
    finally {
        await r.context.close();
    }
});
test('recuperação por e-mail revoga sessões anteriores mesmo sem marcar logout de dispositivos', async ({ browser }) => {
    const recipient = `recovery-${uuidv7()}@e2e.invalid`;
    const r = await register(browser, recipient);
    const contexts: BrowserContext[] = [r.context];
    try {
        const second = await signIn(browser, recipient, password);
        contexts.push(second.context);
        await second.page.waitForURL(state().origin + '/auth/session');
        await body(await second.context.request.get('/api/auth/me'), 200, 'Me');
        const recovery = await browser.newContext({ baseURL: state().origin });
        contexts.push(recovery);
        const page = await recovery.newPage();
        await page.goto(state().origin + '/api/auth/recover');
        await page.locator('input[name="username"]').fill(recipient);
        await page.locator('input[type="submit"],button[type="submit"]').first().click();
        await expect(page.getByText(/receive an email shortly/i)).toBeVisible();
        const link = await emailLink(recipient, 'reset');
        await page.goto(link);
        const next = 'Changed-Password-456!';
        await page.locator('input[name="password-new"]').fill(next);
        await page.locator('input[name="password-confirm"]').fill(next);
        const checkbox = page.locator('input[name="logout-sessions"]');
        if (await checkbox.isVisible())
            await checkbox.uncheck();
        await page.locator('input[type="submit"],button[type="submit"]').first().click();
        await expect.poll(async () => (await r.context.request.get('/api/auth/me')).status()).toBe(401);
        await rejection(await second.context.request.get('/api/auth/me'), 401);
        const old = await signIn(browser, recipient, password);
        contexts.push(old.context);
        await expect(old.page.getByText(/invalid username or password/i)).toBeVisible();
        await rejection(await old.context.request.get('/api/auth/me'), 401);
        const fresh = await signIn(browser, recipient, next);
        contexts.push(fresh.context);
        await fresh.page.waitForURL(state().origin + '/auth/session');
        const me = await body<Me>(await fresh.context.request.get('/api/auth/me'), 200, 'Me');
        expect(me.user_id).toBe(r.me.user_id);
        const replay = await browser.newContext();
        contexts.push(replay);
        const replayPage = await replay.newPage();
        await replayPage.goto(link);
        await expect(replayPage.getByText(/expired|invalid|already used/i).first()).toBeVisible();
    }
    finally {
        await Promise.all(contexts.map(c => c.close()));
    }
});
test('recuperação não revela existência da conta', async ({ browser }) => {
    const context = await browser.newContext({ baseURL: state().origin });
    try {
        const page = await context.newPage();
        await page.goto(state().origin + '/api/auth/recover');
        await page.locator('input[name="username"]').fill(`missing-${uuidv7()}@e2e.invalid`);
        await page.locator('input[type="submit"],button[type="submit"]').first().click();
        await expect(page.getByText(/receive an email shortly/i)).toBeVisible();
        await rejection(await context.request.get('/api/auth/me'), 401);
    }
    finally {
        await context.close();
    }
});
test('falha do Keycloak nega sessão e sinaliza indisponibilidade sem fallback', async ({ alice }) => {
    const { execFile } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const exec = promisify(execFile);
    const current = state();
    const container = current.project + '-keycloak-1';
    try {
        await exec('docker', ['stop', '--time', '5', container]);
        await rejection(await alice.api.send('GET', '/api/auth/me'), 503);
        expect((await alice.api.send('GET', '/ready')).status()).toBe(503);
    }
    finally {
        await exec('docker', ['start', container]);
        await expect.poll(async () => {
            try {
                return (await fetch(current.issuer + '/.well-known/openid-configuration')).status;
            }
            catch {
                return 0;
            }
        }, { timeout: 30000 }).toBe(200);
    }
});
