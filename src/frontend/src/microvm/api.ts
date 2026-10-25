import type { Instance } from "../gen/lazycat/webshell/v1/capability_pb";
import type { SessionRecord } from "./session.ts";

export type MicroVmTarget = { app: string; id: string; name: string };
const apps = new Set(["community.lazycat.app.catflap", "community.lazycat.app.catflap.dev"]);
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export function microVmTarget(instance: Instance): MicroVmTarget | undefined {
  if (!apps.has(instance.providerAppId) || !uuid.test(instance.targetId)) return undefined;
  return { app: instance.providerAppId, id: instance.targetId, name: instance.name || instance.targetId };
}
function base(target: MicroVmTarget) { return new URL(`./api/catflap/${encodeURIComponent(target.app)}/`, window.location.href); }
export function sessionSocket(target: MicroVmTarget, id: string) {
  const url = new URL(`sessions/${encodeURIComponent(id)}/ws`, base(target));
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return new WebSocket(url, "catflap.terminal.v1");
}
export async function createSession(target: MicroVmTarget, record: SessionRecord) {
  const response = await fetch(new URL(`instances/${encodeURIComponent(target.id)}/sessions`, base(target)), {
    method: "POST", credentials: "same-origin", headers: { "content-type": "application/json" },
    body: JSON.stringify({ session_id: record.id, cols: record.cols, rows: record.rows }), signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
  const body = await response.json();
  if (body.session_id !== record.id || body.cols !== record.cols || body.rows !== record.rows) throw new Error("Invalid terminal session response");
}
export async function removeSession(target: MicroVmTarget, id: string) {
  const response = await fetch(new URL(`sessions/${encodeURIComponent(id)}`, base(target)), { method: "DELETE", credentials: "same-origin", signal: AbortSignal.timeout(15000) });
  if (response.status !== 204) {
    const error = new Error(await response.text() || `HTTP ${response.status}`);
    Object.assign(error, { status: response.status });
    throw error;
  }
}
export function sessionStore(target: MicroVmTarget) {
  const key = `neko.microvm.session:${target.app}:${target.id}`;
  return {
    read(): SessionRecord | undefined {
      try { const value = JSON.parse(sessionStorage.getItem(key) || "null");
        if (value && uuid.test(value.id) && Number.isInteger(value.cols) && Number.isInteger(value.rows) && value.cols > 0 && value.cols <= 500 && value.rows > 0 && value.rows <= 500 && (value.exitCode === undefined || Number.isInteger(value.exitCode) && value.exitCode >= 0 && value.exitCode <= 255)) return value;
      } catch { /* Storage may be unavailable. */ }
      return undefined;
    },
    save(value: SessionRecord | undefined) { try { if (value) sessionStorage.setItem(key, JSON.stringify(value)); else sessionStorage.removeItem(key); } catch { /* In-memory identity remains usable. */ } },
  };
}
