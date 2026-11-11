export type RelayCheckFailure = "address" | "timeout" | "connection" | "response";
export class RelayCheckError extends Error {
  readonly reason: RelayCheckFailure;
  constructor(reason: RelayCheckFailure) { super(reason); this.reason = reason; }
}

export function relayHealthUrl(relay: string): string {
  const value = relay.trim();
  let url: URL;
  try { url = new URL(value); } catch { throw new RelayCheckError("address"); }
  // Check the written authority too: URL normalization must not hide credentials,
  // empty query/hash delimiters, dot paths, or backslashes.
  if (!/^(https|wss):\/\/[^/?#\\@]+\/?$/i.test(value)
    || url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    throw new RelayCheckError("address");
  }
  return `wss://${url.host}/healthz`;
}

/** Public, bounded health handshake. Never opens a host/client business channel. */
export async function checkRelayAddress(relay: string, options: { WebSocket?: typeof WebSocket } = {}): Promise<void> {
  const address = relayHealthUrl(relay);
  const Socket = options.WebSocket ?? WebSocket;
  await new Promise<void>((resolve, reject) => {
    let socket: WebSocket;
    try { socket = new Socket(address, "neko.remoted.v1"); }
    catch { reject(new RelayCheckError("connection")); return; }
    let settled = false;
    const finish = (reason?: RelayCheckFailure) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.onmessage = socket.onerror = socket.onclose = null;
      socket.close();
      if (reason) reject(new RelayCheckError(reason)); else resolve();
    };
    const timer = setTimeout(() => finish("timeout"), 8000);
    socket.onerror = socket.onclose = () => finish("connection");
    socket.onmessage = event => {
      // The valid response is tiny. Reject binary and oversized data before parsing.
      if (typeof event.data !== "string" || event.data.length > 1024
        || new TextEncoder().encode(event.data).byteLength > 1024) { finish("response"); return; }
      try {
        const data = JSON.parse(event.data);
        finish(data?.protocol === "neko.remoted.v1" && data?.version === 1 ? undefined : "response");
      } catch { finish("response"); }
    };
  });
}
