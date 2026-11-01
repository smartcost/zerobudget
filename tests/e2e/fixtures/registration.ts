import { expect, type Me } from './auth.js';
import { body, rejection } from './api-client.js';
import { state } from './environment.js';
import type { Browser, BrowserContext } from '@playwright/test';
export const password = 'Signup-Password-123!';
export async function emailLink(recipient: string, kind: 'verify' | 'reset') {
    const current = state();
    let link: string | undefined;
    await expect.poll(async () => {
        const response = await fetch(current.mailpit + '/api/v1/messages');
        const messages = await response.json() as {
            messages: {
                ID: string;
                To: {
                    Address: string;
                }[];
                Subject: string;
            }[];
        };
        for (const m of messages.messages) {
            if (!m.To.some(to => to.Address === recipient) || !(kind === 'verify' ? /verify/i : /reset/i).test(m.Subject))
                continue;
            const detail = await (await fetch(current.mailpit + '/api/v1/message/' + m.ID)).json() as {
                HTML: string;
                Text: string;
            };
            const candidates = [...(detail.HTML.matchAll(/href="([^"]+)"/g))].map(m => m[1].replaceAll('&amp;', '&'));
            link = candidates.find(s => {
                try {
                    const u = new URL(s);
                    return u.origin === new URL(current.issuer).origin && u.pathname === '/realms/zerobudget/login-actions/action-token';
                }
                catch {
                    return false;
                }
            });
            if (link)
                return true;
        }
        return false;
    }, { timeout: 15000, message: 'E-mail de teste não chegou ao Mailpit' }).toBe(true);
    return link!;
}
export async function register(browser: Browser, recipient: string) {
    const context = await browser.newContext({ baseURL: state().origin });
    const page = await context.newPage();
    await page.goto(state().origin + '/api/auth/signup');
    await page.locator('input[name="email"]').fill(recipient);
    await page.locator('input[name="firstName"]').fill('Signup');
    await page.locator('input[name="lastName"]').fill('E2E');
    await page.locator('input[type="submit"],button[type="submit"]').first().click();
    await expect(page.getByText(/verify your email/i).first()).toBeVisible();
    await rejection(await context.request.get('/api/auth/me'), 401);
    const link = await emailLink(recipient, 'verify');
    await page.goto(link);
    await page.locator('input[name="password-new"]').fill(password);
    await page.locator('input[name="password-confirm"]').fill(password);
    await page.locator('input[type="submit"],button[type="submit"]').first().click();
    await page.waitForURL(state().origin + '/auth/session');
    const me = await body<Me>(await context.request.get('/api/auth/me'), 200, 'Me');
    return { context, page, me };
}
export async function signIn(browser: Browser, email: string, value: string) {
    const context = await browser.newContext({ baseURL: state().origin });
    const page = await context.newPage();
    await page.goto(state().origin + '/api/auth/login');
    await page.locator('input[name="username"]').fill(email);
    await page.locator('input[name="password"]').fill(value);
    await page.locator('input[name="login"],button[type="submit"]').first().click();
    return { context, page };
}
