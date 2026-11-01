import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const execute = promisify(execFile);
export const repo = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../..",
);
export type Permission = "budget:read" | "budget:write";
export type State = {
  owner: string;
  project: string;
  dir: string;
  origin: string;
  issuer: string;
  mailpit:string;
  adminUrl: string;
};
export function state(): State {
  const filename = process.env.ZB_E2E_STATE;
  if (!filename) throw new Error("Ambiente E2E não preparado.");
  const value: State = JSON.parse(readFileSync(filename, "utf8"));
  if (
    value.owner !== "zerobudget-e2e-v1" ||
    !/^zb-e2e-[a-f0-9]{12}$/.test(value.project) ||
    path.dirname(value.dir) !== path.join(repo, "tests/e2e/.runs") ||
    path.basename(value.dir) !== value.project ||
    !value.adminUrl.endsWith("/zerobudget_e2e")
  )
    throw new Error("Administração E2E recusada para este ambiente.");
  return value;
}
export async function admin(args: string[]): Promise<void> {
  const current = state();
  const env = {
    ...process.env,
    AUTH_ALLOW_LOCAL_HTTP: "true",
    DATABASE_ADMIN_URL: current.adminUrl,
  };
  // Arguments contain only operation, UUID and permission, never credentials.
  try {
    await execute(path.join(current.dir, "auth_admin"), args, { env });
  } catch {
    throw new Error(`Operação administrativa E2E falhou: ${args[0]}`);
  }
}
