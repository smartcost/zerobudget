import { test, expect } from "../fixtures/auth.js";
import { body, rejection } from "../fixtures/api-client.js";
import type { Me } from "../fixtures/auth.js";
test("login real compartilha cookie com chamadas HTTP e logout revoga a sessão", async ({
  alice,
}) => {
  const me = await body<Me>(
    await alice.api.send("GET", "/api/auth/me"),
    200,
    "Me",
  );
  expect(me.user_id).toBe(alice.me.user_id);
  expect(me.expires_at).toBeGreaterThan(Math.floor(Date.now() / 1000));
  expect((await alice.api.send("POST", "/api/auth/logout")).status()).toBe(204);
  await rejection(await alice.api.send("GET", "/api/auth/me"), 401);
});
test("sem sessão a API não aceita bearer arbitrário", async ({ request }) => {
  await rejection(await request.get("/api/auth/me"), 401);
  await rejection(
    await request.get("/api/cycles", {
      headers: { Authorization: "Bearer invalid-test-token" },
    }),
    401,
  );
});
test("início OIDC usa PKCE S256 e callback sem cookie correspondente é negado", async ({
  request,
}) => {
  const response = await request.get("/api/auth/login", { maxRedirects: 0 });
  expect(response.status()).toBe(303);
  const location = new URL(response.headers().location);
  expect(location.searchParams.get("code_challenge_method")).toBe("S256");
  expect(location.searchParams.get("code_challenge")).toBeTruthy();
  expect(location.searchParams.get("nonce")).toBeTruthy();
  // This request has its own pending cookie, but the state is deliberately invalid.
  await rejection(
    await request.get("/api/auth/callback?code=invalid&state=invalid", {
      maxRedirects: 0,
    }),
    401,
  );
});
