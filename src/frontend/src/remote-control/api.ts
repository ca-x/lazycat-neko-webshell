export type Permission = "read_only" | "operate";
export type RemoteResource = { id: string; name: string; backend: string; terminal: boolean; herdr: boolean; files: boolean };
export type ConnectionKind = "local" | "relay" | "connecting" | "direct";
export type RemoteDevice = {
  id: string; name: string; fingerprint: string; permission: Permission;
  resources: RemoteResource[]; online: boolean; connections: number; rttMs: number | null; connectionTypes?: ConnectionKind[];
};
export type PendingDevice = { id: string; name: string; fingerprint: string; expiresInSeconds: number };
export type RemoteState = {
  enabled: boolean; relay: string; has_registration_key?: boolean; status: string; error: string; relayRttMs: number | null;
  devices: RemoteDevice[]; pending: PendingDevice[];
  hostFingerprint?: string;
  operations?: { inFlightUnknown: number; active: RemoteOperation[]; recent: RemoteOperation[] };
};
export type RemoteOperation = { id: string; deviceId: string; operation: string; outcomeUnknown: boolean };
export type ResourceChoice = { session: string; root: string | null };
export type RemoteAction =
  | { action: "configure"; enabled: boolean; relay: string; registrationKey?: string }
  | { action: "approve"; pending: string; permission: Permission; resources: ResourceChoice[] }
  | { action: "reject"; pending: string }
  | { action: "permission"; id: string; permission: Permission }
  | { action: "disconnect" | "revoke"; id: string };
export type Invitation = { code: string; expiresInSeconds: number };

export class RemoteApiError extends Error {
  readonly status: number;
  readonly code: string | undefined;

  constructor(status: number, code: string | undefined, message: string) {
    super(message);
    this.name = "RemoteApiError";
    this.status = status;
    this.code = code;
  }

  get authorizationLost(): boolean {
    return this.status === 401 || this.status === 403 || this.code === "AUTHORIZATION_FAILED";
  }
}

async function request<T>(action?: RemoteAction | { action: "invite" }): Promise<T> {
  const response = await fetch(new URL("./api/remoted", window.location.href), {
    method: action ? "POST" : "GET", credentials: "same-origin", cache: "no-store",
    headers: { "x-neko-remoted-request": "1", ...(action ? { "content-type": "application/json" } : {}) },
    body: action ? JSON.stringify(action) : undefined,
    signal: AbortSignal.timeout(15000),
  });
  const payload = await response.json().catch(() => undefined);
  if (!response.ok) throw new RemoteApiError(response.status, payload?.code, payload?.error || `HTTP ${response.status}`);
  if (!payload) throw new Error("Invalid remote control response");
  if (action?.action !== "invite" && (typeof payload.enabled !== "boolean" || typeof payload.relay !== "string" || typeof payload.status !== "string" || !Array.isArray(payload.devices) || !Array.isArray(payload.pending))) {
    throw new Error("Invalid remote control status response");
  }
  return payload as T;
}
export const remoteApi = {
  get: () => request<RemoteState>(),
  act: (action: RemoteAction) => request<RemoteState>(action),
  invite: () => request<Invitation>({ action: "invite" }),
};
