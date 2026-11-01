import {
  test as base,
  expect,
  type BrowserContext,
  type Page,
} from "@playwright/test";
import { ApiClient, body } from "./api-client.js";
import { state, admin, type Permission } from "./environment.js";
export { expect };
export type Account = "alice" | "bob" | "denied" | "reader" | "writer";
export type Me = {
  user_id: string;
  csrf_token: string;
  permissions: Permission[];
  issuer: string;
  subject: string;
  expires_at: number;
};
export type Actor = {
  account: Account;
  context: BrowserContext;
  page: Page;
  api: ApiClient;
  me: Me;
};
const roles: Record<Account, Permission[]> = {
  alice: ["budget:read", "budget:write"],
  bob: ["budget:read", "budget:write"],
  denied: [],
  reader: ["budget:read"],
  writer: ["budget:write"],
};
export const test = base.extend<{
  login: (name: Account, permissions?: Permission[]) => Promise<Actor>;
  alice: Actor;
}>({
  login: async ({ browser }, use) => {
    const actors: Actor[] = [];
    const contexts: BrowserContext[] = [];
    try {
      await use(async (account, permissions = roles[account]) => {
        const current = state();
        const context = await browser.newContext({ baseURL: current.origin });
        contexts.push(context);
        const page = await context.newPage();
        await page.goto(current.origin + "/api/auth/login");
        await page.locator('input[name="username"]').fill(`${account}@e2e.invalid`);
        await page.locator('input[name="password"]').fill("E2e-Password-123!");
        await page.locator('input[name="login"],button[type="submit"]').first().click();
        await page.waitForURL(current.origin + "/auth/session");
        let me = await body<Me>(
          await context.request.get("/api/auth/me"),
          200,
          "Me",
        );
        expect(me.issuer).toBe(current.issuer);
        expect(
          (await context.cookies()).some(
            (c) =>
              c.name === "zb_session" && c.httpOnly && c.sameSite === "Lax",
          ),
        ).toBe(true);
        for (const permission of [
          "budget:read",
          "budget:write",
        ] as Permission[]) {
          await admin([
            permissions.includes(permission) ? "grant" : "revoke",
            me.user_id,
            permission,
          ]);
        }
        me = await body<Me>(
          await context.request.get("/api/auth/me"),
          200,
          "Me",
        );
        expect([...me.permissions].sort()).toEqual([...permissions].sort());
        const actor = {
          account,
          context,
          page,
          api: new ApiClient(context.request, me.csrf_token),
          me,
        };
        actors.push(actor);
        return actor;
      });
    } finally {
      // A failed block/revoke test must not prevent the next UI execution from logging in.
      for (const actor of actors) await admin(["enable", actor.me.user_id]);
      await Promise.all(contexts.map((context) => context.close()));
    }
  },
  alice: async ({ login }, use) => {
    await use(await login("alice"));
  },
});
