export type SessionRecord = { id: string; cols: number; rows: number; exitCode?: number };
export type SessionStatus = { state: "connecting" | "ready" | "disconnected" | "exited" | "error"; gap?: boolean; code?: number; message?: string; retryable?: boolean };
export type TerminalEvent =
  | { type: "ready"; session_id: string; cols: number; rows: number; replay: false; output_gap: boolean }
  | { type: "process-exit"; exit_code: number }
  | { type: "error"; message: string; fatal: boolean; retryable: boolean };
export function parseEvent(text: string, sessionId: string): TerminalEvent {
  if (text.length > 16 * 1024) throw new Error("Invalid terminal event");
  const event = JSON.parse(text);
  if (event.type === "ready" && event.session_id === sessionId && event.replay === false && typeof event.output_gap === "boolean" && size(event.cols) && size(event.rows)) return event;
  if (event.type === "process-exit" && Number.isInteger(event.exit_code) && event.exit_code >= 0 && event.exit_code <= 255) return event;
  if (event.type === "error" && typeof event.message === "string" && typeof event.fatal === "boolean" && typeof event.retryable === "boolean") return event;
  throw new Error("Invalid terminal event");
}
function size(value: unknown): value is number { return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 500; }
export function terminalSize(cols: number, rows: number) { return { cols: Math.max(1, Math.min(500, Math.trunc(cols) || 80)), rows: Math.max(1, Math.min(500, Math.trunc(rows) || 24)) }; }

export function createMicroVmSession(options: {
  record?: SessionRecord;
  save: (record: SessionRecord | undefined) => void;
  create: (record: SessionRecord) => Promise<void>;
  remove: (id: string) => Promise<void>;
  socket: (id: string) => WebSocket;
  output: (bytes: Uint8Array) => void;
  changed: (status: SessionStatus) => void;
}) {
  let record = options.record;
  let socket: WebSocket | undefined;
  let stopped = false, ready = false, fatal = false, creating = false;
  let retries = 0;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let readyTimer: ReturnType<typeof setTimeout> | undefined;
  let grid = terminalSize(record?.cols || 80, record?.rows || 24);
  const cancelTimers = () => { clearTimeout(retryTimer); clearTimeout(readyTimer); };
  const detach = () => { cancelTimers(); ready = false; const current = socket; socket = undefined; current?.close(); };
  function attach() {
    if (stopped || creating || !record || record.exitCode !== undefined) return;
    detach(); fatal = false;
    options.changed({ state: "connecting" });
    const current = options.socket(record.id); socket = current; current.binaryType = "arraybuffer";
    readyTimer = setTimeout(() => { if (socket === current) current.close(); }, 15000);
    current.addEventListener("message", event => {
      if (socket !== current || stopped || !record) return;
      try {
        if (event.data instanceof ArrayBuffer) { options.output(new Uint8Array(event.data)); return; }
        if (typeof event.data !== "string") throw new Error("Invalid terminal frame");
        const value = parseEvent(event.data, record.id);
        if (value.type === "ready") {
          clearTimeout(readyTimer); retries = 0; ready = true;
          options.changed({ state: "ready", gap: value.output_gap });
          current.send(JSON.stringify({ type: "resize", ...grid }));
        } else if (value.type === "process-exit") {
          record = { ...record, exitCode: value.exit_code }; options.save(record);
          fatal = true; ready = false; clearTimeout(readyTimer);
          options.changed({ state: "exited", code: value.exit_code });
          current.close();
        } else {
          fatal = value.fatal || !value.retryable; ready = false;
          options.changed({ state: "error", message: value.message, retryable: !fatal });
          current.close();
        }
      } catch (error) {
        fatal = true; ready = false;
        options.changed({ state: "error", message: String(error), retryable: false }); current.close();
      }
    });
    current.addEventListener("close", () => {
      if (socket !== current || stopped) return;
      socket = undefined; ready = false; clearTimeout(readyTimer);
      if (fatal || record?.exitCode !== undefined) return;
      options.changed({ state: "disconnected", retryable: true });
      // Retry attach only, never repeat create or invent a successful process exit.
      if (retries < 5) retryTimer = setTimeout(attach, Math.min(4000, 500 * 2 ** retries++));
    });
    current.addEventListener("error", () => { /* close owns reconnection */ });
  }
  async function connect(cols: number, rows: number) {
    grid = terminalSize(cols, rows);
    if (stopped || creating || socket) return;
    if (record?.exitCode !== undefined) { options.changed({ state: "exited", code: record.exitCode }); return; }
    if (!record) {
      record = { id: crypto.randomUUID(), ...grid };
      // Persist before the request. An ambiguous response must never create another shell.
      options.save(record); creating = true; options.changed({ state: "connecting" });
      try { await options.create(record); }
      catch (error) { options.changed({ state: "error", message: String(error), retryable: true }); return; }
      finally { creating = false; }
    }
    attach();
  }
  return {
    connect,
    reconnect: () => { if (!creating) { retries = 0; attach(); } },
    send: (data: string) => {
      if (!ready || socket?.readyState !== WebSocket.OPEN) return false;
      socket.send(new TextEncoder().encode(data)); return true;
    },
    resize: (cols: number, rows: number) => {
      grid = terminalSize(cols, rows);
      if (!ready || socket?.readyState !== WebSocket.OPEN) return false;
      socket.send(JSON.stringify({ type: "resize", ...grid })); return true;
    },
    connected: () => ready && !stopped,
    newSession: async () => {
      if (creating) return;
      try { await end(); }
      catch (error) {
        // Only explicit New may forget an inaccessible/expired session. Do not
        // claim its cleanup succeeded; a 504 or ambiguous DELETE keeps the ID.
        if (!(error instanceof Error) || !("status" in error) || error.status !== 403) throw error;
        record = undefined; options.save(undefined);
      }
      await connect(grid.cols, grid.rows);
    },
    end,
    stop: () => { stopped = true; detach(); },
  };
  async function end() {
    if (creating) throw new Error("Terminal creation is still pending");
    detach();
    if (record) await options.remove(record.id);
    record = undefined; options.save(undefined);
    options.changed({ state: "exited" });
  }
}
