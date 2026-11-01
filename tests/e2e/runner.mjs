import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
  existsSync,
  readdirSync,
  copyFileSync,
  chmodSync,
  openSync,
  closeSync,
  rmSync,
} from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import net from "node:net";
import { randomUUID, createHash } from "node:crypto";

export const repo = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
export const home = path.join(repo, "tests/e2e");
export const runs = path.join(home, ".runs");
export const browsers = path.join(home, ".cache/browsers");
const target = path.resolve(repo, process.env.CARGO_TARGET_DIR || "target");
const commonEnv = { ...process.env, PLAYWRIGHT_BROWSERS_PATH: browsers };
const command = process.argv[2] || "help";
let active;
let apiProcess;
let stopping = false;

function execute(program, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, {
      cwd: repo,
      env: commonEnv,
      stdio: "inherit",
      ...options,
    });
    child.once("error", reject);
    child.once("exit", (code, signal) =>
      code === 0
        ? resolve()
        : reject(new Error(`${program} terminou com ${code ?? signal}`)),
    );
  });
}
function save(state) {
  writeFileSync(
    path.join(state.dir, "state.json"),
    JSON.stringify(state, null, 2),
  );
}
function validState(state) {
  return (
    state?.owner === "zerobudget-e2e-v1" &&
    /^zb-e2e-[a-f0-9]{12}$/.test(state.project) &&
    path.dirname(state.dir) === runs &&
    path.basename(state.dir) === state.project &&
    state.apiExecutable === path.join(state.dir, "obz-api")
  );
}
export function loadState(filename = process.env.ZB_E2E_STATE) {
  let candidate = filename;
  if (!candidate) {
    const states = existsSync(runs)
      ? readdirSync(runs)
          .map((p) => path.join(runs, p, "state.json"))
          .filter(existsSync)
      : [];
    const running = states.filter(
      (p) => JSON.parse(readFileSync(p)).status === "running",
    );
    if (running.length > 1)
      throw new Error(
        "Há vários ambientes E2E. Selecione o manifesto com ZB_E2E_STATE.",
      );
    candidate = running[0];
  }
  if (!candidate)
    throw new Error("Nenhum ambiente E2E ativo. Execute ./scripts/e2e up.");
  const state = JSON.parse(readFileSync(candidate));
  if (!validState(state))
    throw new Error("Manifesto E2E inválido; nenhuma limpeza será executada.");
  return state;
}
function composeEnv(state) {
  return {
    ...commonEnv,
    E2E_POSTGRES_PORT: String(state.ports.POSTGRES),
    E2E_REDIS_PORT: String(state.ports.REDIS),
    E2E_KEYCLOAK_PORT: String(state.ports.KEYCLOAK),
    E2E_KEYCLOAK_REALM:path.join(state.dir,"realm.json"),
    E2E_MAILPIT_PORT:String(state.ports.MAILPIT),
  };
}
function composeArgs(state, args) {
  return [
    "compose",
    "--project-directory",
    repo,
    "-p",
    state.project,
    "--env-file",
    path.join(state.dir, ".env"),
    "-f",
    path.join(repo, "config/e2e/compose.yaml"),
    ...args,
  ];
}
function cleanEnvironment() {
  const env = { ...commonEnv };
  delete env.DATABASE_ADMIN_URL;
  delete env.DATABASE_RUNTIME_ROLE;
  return env;
}
export async function admin(state, args) {
  if (!validState(state) || !state.adminUrl.endsWith("/zerobudget_e2e"))
    throw new Error("Administração restrita ao ambiente E2E.");
  await execute(path.join(state.dir, "auth_admin"), args, {
    env: {
      ...cleanEnvironment(),
      AUTH_ALLOW_LOCAL_HTTP: "true",
      DATABASE_ADMIN_URL: state.adminUrl,
    },
  });
}
async function portFree(port) {
  await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", () =>
      reject(
        new Error(
          `Porta E2E ${port} ocupada. Configure ZB_E2E_API_PORT/KEYCLOAK_PORT/POSTGRES_PORT/REDIS_PORT/MAILPIT_PORT; nenhum processo foi encerrado.`,
        ),
      ),
    );
    server.listen(port, "127.0.0.1", () => server.close(resolve));
  });
}
async function waitHttp(url, check, timeout = 120000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (apiProcess?.exitCode !== null && apiProcess?.exitCode !== undefined)
      throw new Error(
        "API encerrou na inicialização; consulte api.log do run.",
      );
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (await check(response)) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Serviço não ficou pronto: ${new URL(url).origin}`);
}
async function setup(force = false) {
  if (
    Number(process.versions.node.split(".")[0]) < 22 ||
    Number(process.versions.node.split(".")[0]) >= 25
  )
    throw new Error("Use Node 22 a 24.");
  const fingerprint = createHash("sha256")
    .update(readFileSync(path.join(home, "package-lock.json")))
    .update(readFileSync(path.join(home, "package.json")))
    .digest("hex");
  const marker = path.join(home, ".cache/dependencies.sha256");
  if (
    force ||
    !existsSync(
      path.join(home, "node_modules/@playwright/test/package.json"),
    ) ||
    !existsSync(marker) ||
    readFileSync(marker, "utf8") !== fingerprint
  ) {
    await execute("npm", [
      "ci",
      "--prefix",
      home,
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
    ]);
    mkdirSync(path.dirname(marker), { recursive: true });
    writeFileSync(marker, fingerprint);
  }
  const { chromium } = await import(
    "./node_modules/@playwright/test/index.mjs"
  );
  process.env.PLAYWRIGHT_BROWSERS_PATH = browsers;
  // Playwright reads the browser directory at import time; child CLI uses the same env.
  if (force || !existsSync(chromium.executablePath()))
    await execute(process.execPath, [
      path.join(home, "node_modules/@playwright/test/cli.js"),
      "install",
      "chromium",
    ]);
  await execute("npm", ["--prefix", home, "run", "typecheck"]);
  const mcpConfig = path.join(home, ".cache/mcp-client.json");
  mkdirSync(path.dirname(mcpConfig), { recursive: true });
  writeFileSync(
    mcpConfig,
    JSON.stringify(
      {
        mcpServers: {
          "zerobudget-playwright": {
            command: path.join(repo, "scripts/e2e"),
            args: ["mcp"],
          },
        },
      },
      null,
      2,
    ),
  );
}
async function start() {
  const names = ["API", "KEYCLOAK", "POSTGRES", "REDIS", "MAILPIT"];
  const defaults = [3100, 8180, 5540, 6480, 8125];
  const ports = Object.fromEntries(
    names.map((name, i) => {
      const value = Number(process.env[`ZB_E2E_${name}_PORT`] || defaults[i]);
      if (!Number.isInteger(value) || value < 1024 || value > 65535)
        throw new Error("Porta E2E inválida.");
      return [name, value];
    }),
  );
  if (new Set(Object.values(ports)).size !== names.length)
    throw new Error("As portas E2E devem ser distintas.");
  for (const port of Object.values(ports)) await portFree(port);
  await execute("docker", ["compose", "version"]);
  await execute("cargo", [
    "build",
    "-p",
    "obz-api",
    "--bin",
    "obz-api",
    "--example",
    "migrate",
    "--target-dir",
    target,
  ]);
  await execute("cargo", [
    "build",
    "-p",
    "obz-infrastructure",
    "--example",
    "auth_admin",
    "--target-dir",
    target,
  ]);
  const project = `zb-e2e-${randomUUID().replaceAll("-", "").slice(0, 12)}`;
  const dir = path.join(runs, project);
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  active = {
    owner: "zerobudget-e2e-v1",
    project,
    dir,
    apiExecutable: path.join(dir, "obz-api"),
    status: "starting",
    ports,
    origin: `http://localhost:${ports.API}`,
    issuer: `http://localhost:${ports.KEYCLOAK}/realms/zerobudget`,
    mailpit:`http://localhost:${ports.MAILPIT}`,
    adminUrl: `postgres://zb_e2e_admin:e2e-admin-password@127.0.0.1:${ports.POSTGRES}/zerobudget_e2e`,
  };
  save(active);
  const realm=JSON.parse(readFileSync(path.join(repo,"config/keycloak/realm.json"),"utf8"));
  realm.clients[0].secret="e2e-client-secret";
  realm.clients[0].redirectUris=[active.origin+"/api/auth/callback"];
  realm.clients[0].webOrigins=[active.origin];
  realm.smtpServer.from="no-reply@e2e.invalid";
  realm.users=["alice","bob","denied","reader","writer"].map((name,i)=>({id:`00000000-0000-4000-8000-${String(i+1).padStart(12,"0")}`,username:`${name}@e2e.invalid`,email:`${name}@e2e.invalid`,firstName:name,lastName:"E2E",emailVerified:true,enabled:true,credentials:[{type:"password",value:"E2e-Password-123!",temporary:false}]}));
  writeFileSync(path.join(dir,"realm.json"),JSON.stringify(realm,null,2));
  writeFileSync(path.join(dir,".env"),`E2E_POSTGRES_PORT=${ports.POSTGRES}
E2E_KEYCLOAK_PORT=${ports.KEYCLOAK}
E2E_REDIS_PORT=${ports.REDIS}
E2E_MAILPIT_PORT=${ports.MAILPIT}
E2E_KEYCLOAK_REALM=${path.join(dir,"realm.json")}
`);
  for (const [source, dest] of [
    ["obz-api", "obz-api"],
    ["examples/auth_admin", "auth_admin"],
    ["examples/migrate", "migrate"],
  ]) {
    copyFileSync(path.join(target, "debug", source), path.join(dir, dest));
    chmodSync(path.join(dir, dest), 0o700);
  }
  await execute(
    "docker",
    composeArgs(active, ["up", "-d", "--build", "--wait", "--wait-timeout", "120"]),
    { env: composeEnv(active) },
  );
  await waitHttp(
    active.issuer + "/.well-known/openid-configuration",
    async (response) =>
      response.ok && (await response.json()).issuer === active.issuer,
  );
  await waitHttp(
    active.issuer + "/protocol/openid-connect/certs",
    async (response) => response.ok && (await response.json()).keys.length > 0,
  );
  await execute(path.join(dir, "migrate"), [], {
    env: {
      ...cleanEnvironment(),
      AUTH_ALLOW_LOCAL_HTTP: "true",
      DATABASE_ADMIN_URL: active.adminUrl,
      DATABASE_RUNTIME_ROLE: "zb_e2e_runtime",
    },
  });
  const apiEnv = {
    ...cleanEnvironment(),
    ZB_E2E_RUN_ID: project,
    API_BIND: `127.0.0.1:${ports.API}`,
    API_PUBLIC_URL: active.origin,
    OIDC_ISSUER: active.issuer,
    OIDC_CLIENT_ID: "zerobudget",
    OIDC_CLIENT_SECRET: "e2e-client-secret",
    AUTH_ALLOW_LOCAL_HTTP: "true",
    AUTH_REDIS_TIMEOUT_MS: "3000",
    AUTH_MAX_PENDING: "128",
    AUTH_MAX_SESSIONS: "128",
    DATABASE_TIMEOUT_MS: "3000",
    DATABASE_MAX_CONNECTIONS: "10",
    AUTH_SESSION_BACKEND: "redis",
    AUTH_REDIS_URL: `redis://127.0.0.1:${ports.REDIS}/0`,
    AUTH_REDIS_NAMESPACE: project,
    DATABASE_URL: `postgres://zb_e2e_runtime:e2e-runtime-password@127.0.0.1:${ports.POSTGRES}/zerobudget_e2e`,
  };
  const log = openSync(path.join(dir, "api.log"), "a", 0o600);
  apiProcess = spawn(active.apiExecutable, [], {
    cwd: repo,
    env: apiEnv,
    stdio: ["ignore", log, log],
    detached: true,
  });
  closeSync(log);
  await once(apiProcess, "spawn");
  active.apiPid = apiProcess.pid;
  save(active);
  await waitHttp(active.origin + "/health", (r) => r.status === 204);
  active.status = "running";
  save(active);
  console.log(`Ambiente E2E pronto: ${active.origin} (${project})`);
  return active;
}
async function clean(state) {
  if (!validState(state)) throw new Error("Manifesto E2E inválido.");
  if (state.apiPid) {
    try {
      const args = readFileSync(`/proc/${state.apiPid}/cmdline`, "utf8").split(
        "\0",
      );
      if (args[0] === state.apiExecutable) {
        process.kill(state.apiPid, "SIGTERM");
        for (let i = 0; i < 30; i++) {
          try {
            process.kill(state.apiPid, 0);
          } catch {
            break;
          }
          await new Promise((r) => setTimeout(r, 100));
        }
        try {
          const current = readFileSync(
            `/proc/${state.apiPid}/cmdline`,
            "utf8",
          ).split("\0");
          if (current[0] === state.apiExecutable)
            process.kill(state.apiPid, "SIGKILL");
        } catch {}
      }
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ESRCH") throw error;
    }
  }
  await execute(
    "docker",
    composeArgs(state, [
      "down",
      "--volumes",
      "--remove-orphans",
      "--timeout",
      "5",
    ]),
    { env: composeEnv(state) },
  );
  state.status = "stopped";
  save(state);
  for (const file of ["obz-api", "auth_admin", "migrate"])
    rmSync(path.join(state.dir, file), { force: true });
  console.log(`Recursos E2E removidos: ${state.project}`);
}
async function shutdown(signal) {
  if (stopping) return;
  stopping = true;
  try {
    if (active) await clean(active);
  } catch (error) {
    console.error(error.message);
  }
  process.exit(signal === "SIGINT" ? 130 : 143);
}
async function main() {
  if (command === "help") {
    console.log(
      "Uso: ./scripts/e2e setup | test [filtros Playwright] | verify | ui | up | clean | mcp | mcp-check",
    );
    return;
  }
  if (command === "clean") {
    if (existsSync(runs))
      for (const dir of readdirSync(runs)) {
        const file = path.join(runs, dir, "state.json");
        if (!existsSync(file)) continue;
        const state = loadState(file);
        if (state.status !== "stopped") await clean(state);
      }
    return;
  }
  process.env.PLAYWRIGHT_BROWSERS_PATH = browsers;
  if (command === "mcp") {
    const { serve } = await import("./mcp-server.mjs");
    serve();
    return;
  }
  if (command === "setup") {
    await setup(true);
    return;
  }
  if (!["test", "verify", "ui", "up", "mcp-check"].includes(command))
    throw new Error("Comando E2E desconhecido. Use ./scripts/e2e help.");
  await setup();
  process.on("SIGINT", () => void shutdown("SIGINT"));
  process.on("SIGTERM", () => void shutdown("SIGTERM"));
  try {
    const state = await start();
    if (command === "up") {
      apiProcess.unref();
      console.log(`Estado: ${path.join(state.dir, "state.json")}`);
      console.log(
        "Ambiente mantido para MCP. Encerre com ./scripts/e2e clean.",
      );
      return;
    }
    const stateEnv = {
      ...cleanEnvironment(),
      ZB_E2E_STATE: path.join(state.dir, "state.json"),
    };
    if (command === "mcp-check")
      await execute(process.execPath, [path.join(home, "mcp-probe.mjs")], {
        env: stateEnv,
      });
    else
      await execute(
        process.execPath,
        [
          path.join(home, "node_modules/@playwright/test/cli.js"),
          "test",
          "--config",
          path.join(home, "playwright.config.ts"),
          ...(command === "ui" ? ["--ui"] : process.argv.slice(3)),
        ],
        { env: stateEnv },
      );
    if (command === "verify") {
      const testEnv = {
        ...stateEnv,
        AUTH_TEST_DATABASE_URL: state.adminUrl,
        AUTH_TEST_RUNTIME_DATABASE_URL: `postgres://zb_e2e_runtime:e2e-runtime-password@127.0.0.1:${state.ports.POSTGRES}/zerobudget_e2e`,
        AUTH_TEST_REDIS_URL: `redis://127.0.0.1:${state.ports.REDIS}/0`,
        ZB_MIGRATE_BIN: path.join(state.dir, "migrate"),
      };
      await execute("cargo", ["test", "--workspace", "--locked", "--", "--include-ignored", "--test-threads=2"], { env: testEnv });
      await execute("python3", [path.join(repo, "tests/operations/backup_restore.py")], { env: testEnv });
    }
  } catch (error) {
    if (active) {
      const logs = spawnSync(
        "docker",
        composeArgs(active, ["logs", "--no-color"]),
        { cwd: repo, env: composeEnv(active), encoding: "utf8" },
      );
      writeFileSync(
        path.join(active.dir, "services-private.log"),
        logs.stdout || "",
      );
      console.error(`Diagnóstico local: ${active.dir}/services-private.log`);
    }
    throw error;
  } finally {
    if (active && !(command === "up" && active.status === "running"))
      await clean(active);
  }
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
