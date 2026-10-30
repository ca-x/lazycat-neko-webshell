import type { MessageKey } from "./i18n";
import type { SessionBackendId, SessionBackendsState, SessionBackendInfo } from "./types";
import { escapeAttr, escapeHtml } from "./utils.ts";

export type SessionMode = SessionBackendId;

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

const WEB_SHELL_BACKEND: SessionBackendInfo = {
  id: "webshell",
  label: "WebShell native",
  available: true,
};

export function selectableSessionBackends(state: SessionBackendsState | undefined): SessionBackendInfo[] {
  const backends = state?.backends ?? [WEB_SHELL_BACKEND];
  return backends.filter((backend) => backend.available || backend.id === "webshell");
}

export function normalizeSessionMode(value: unknown): SessionMode {
  return value === "hc" || value === "herdr" || value === "zellij" || value === "ssh" ? value : "webshell";
}

export function sessionBackendInstalled(
  state: SessionBackendsState | undefined,
  mode: SessionMode,
): boolean {
  return selectableSessionBackends(state).some((backend) => backend.id === mode);
}

export function sessionBackendIsSelectable(
  state: SessionBackendsState | undefined,
  mode: SessionMode,
): boolean {
  if (!state && mode === "webshell") return true;
  return selectableSessionBackends(state).some((backend) => backend.id === mode);
}

export function preferredAvailableBackend(
  state: SessionBackendsState | undefined,
  preferred: SessionMode,
  sshTarget = false,
): SessionMode {
  // Keep HC intent even when unavailable; creation must fail explicitly.
  if (preferred === "hc") return preferred;
  if (sshTarget && (!state || sessionBackendIsSelectable(state, "ssh"))) return "ssh";
  return sessionBackendIsSelectable(state, preferred) ? preferred : "webshell";
}

export class HcBackendUnavailableError extends Error {
  constructor() {
    super("HC is unavailable for this target. Select an available backend explicitly to create a new session.");
    this.name = "HcBackendUnavailableError";
  }
}

export function resolveNewSessionBackend(
  state: SessionBackendsState | undefined,
  preferred: SessionMode,
  requested?: SessionMode,
): SessionMode {
  const mode = requested === "hc" || (requested && sessionBackendIsSelectable(state, requested))
    ? requested : preferredAvailableBackend(state, preferred);
  if (mode === "hc" && !sessionBackendIsSelectable(state, mode)) throw new HcBackendUnavailableError();
  return mode;
}

export function sessionBackendLabel(id: SessionBackendId, fallback: string, tr: Translate): string {
  if (id === "webshell") return tr("backend.webshell");
  if (id === "hc") return tr("backend.hc");
  if (id === "herdr") return tr("backend.herdr");
  if (id === "zellij") return tr("backend.zellij");
  return fallback;
}

export function sessionBackendSupportsTerminalTransfer(backend: SessionBackendInfo | undefined): boolean {
  return Boolean(backend?.supportsTerminalTransfer ?? backend?.supports_terminal_transfer);
}

export function renderSessionBackendSelectOptions(
  backends: SessionBackendInfo[],
  tr: Translate,
  preferred?: SessionMode,
): string {
  const choices = preferred === "hc" && !backends.some((backend) => backend.id === "hc")
    ? [...backends, { id: "hc" as const, label: tr("backend.hcUnavailable"), available: false }]
    : backends;
  return choices
    .map((backend) => `<option value="${escapeAttr(backend.id)}"${backend.id === "hc" && !backend.available ? " disabled" : ""}>${escapeHtml(backend.id === "hc" && !backend.available ? tr("backend.hcUnavailable") : sessionBackendLabel(backend.id, backend.label, tr))}</option>`)
    .join("");
}

// Missing backend metadata is the legacy native workspace format only.
export function restoredSessionBackend(value: unknown): SessionMode {
  if (value === undefined) return "webshell";
  const mode = normalizeSessionMode(value);
  if (value !== mode) throw new Error(`Unsupported session backend: ${String(value)}`);
  return mode;
}

export function sessionBackendSupportsLocalSplit(backend: SessionBackendId): boolean {
  return backend === "webshell" || backend === "hc" || backend === "ssh";
}
