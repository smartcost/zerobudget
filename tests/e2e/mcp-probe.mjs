import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { loadState, repo } from "./runner.mjs";
import { writeFileSync } from "node:fs";
import path from "node:path";
const state = loadState();
const client = new Client({ name: "zerobudget-e2e", version: "0.1.0" });
const transport = new StdioClientTransport({
  command: path.join(repo, "scripts/e2e"),
  args: ["mcp"],
  env: process.env,
  stderr: "pipe",
});
try {
  await client.connect(transport);
  const { tools } = await client.listTools();
  if (!tools.some((t) => t.name === "browser_run_code_unsafe"))
    throw new Error("MCP não disponibilizou browser_run_code_unsafe.");
  const result = await client.callTool({
    name: "browser_run_code_unsafe",
    arguments: {
      code: `async (page) => {
    await page.goto(${JSON.stringify(state.origin + "/api/auth/login")});
    const connector=page.getByRole('link',{name:/Log in with Email/i});
    if(await connector.isVisible())await connector.click();
    await page.locator('input[name="login"]').fill('alice@e2e.invalid');
    await page.locator('input[name="password"]').fill('e2e-password');
    await page.getByRole('button',{name:/^Log\s*in$/i}).click();
    await page.getByRole('button',{name:/Grant Access|Approve/i}).click();
    await page.waitForURL(${JSON.stringify(state.origin + "/auth/session")});
    const response=await page.context().request.get(${JSON.stringify(state.origin + "/api/auth/me")});
    if(response.status()!==200)throw new Error('Sessão HTTP não foi compartilhada');
    const me=await response.json();
    if(me.issuer!==${JSON.stringify(state.issuer)})throw new Error('Issuer divergente');
    return {authenticated:true,sessionStatus:response.status(),financialGrants:me.permissions.length};
  }`,
    },
  });
  if (result.isError) {
    writeFileSync(
      path.join(state.dir, "mcp-failure-private.json"),
      JSON.stringify(result),
      { mode: 0o600 },
    );
    throw new Error(
      "Prova MCP falhou; consulte mcp-failure-private.json no run local.",
    );
  }
  const text = result.content
    .filter((block) => block.type === "text")
    .map((block) => block.text)
    .join("\n");
  const section = text.match(/### Result\s*\n([\s\S]*?)(?=\n### |$)/)?.[1];
  let summary;
  try {
    summary = JSON.parse(
      (section || "").replace(/^```(?:json)?\s*|\s*```$/g, ""),
    );
  } catch {}
  if (
    summary?.authenticated !== true ||
    summary?.sessionStatus !== 200 ||
    summary?.financialGrants !== 0
  ) {
    writeFileSync(
      path.join(state.dir, "mcp-failure-private.json"),
      JSON.stringify(result),
      { mode: 0o600 },
    );
    throw new Error(
      "MCP não retornou a prova estruturada de login; consulte o diagnóstico privado do run.",
    );
  }
  console.log(
    `Playwright MCP: ${tools.length} ferramentas, login Dex real e cookie compartilhado confirmados.`,
  );
} finally {
  await client.close();
}
