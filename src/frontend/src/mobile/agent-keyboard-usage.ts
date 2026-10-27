import type { MobileAgentKeyboardEntry, MobileKeyboardAgent } from "./agent-keymap.ts";

const counts = new Map<string, number>();
const MAX_COUNT = 1_000_000;
let generation = 0;
let userId: string | undefined;

type Usage = { agent: MobileKeyboardAgent; group: string; shortcut: string; count: number };

function usageKey(agent: MobileKeyboardAgent, group: string, value: string): string {
  return `${agent}:${group}:${value}`;
}

function validUsage(value: unknown): value is Usage {
  if (!value || typeof value !== "object") return false;
  const usage = value as Partial<Usage>;
  return ["codex", "claude", "opencode", "pi"].includes(usage.agent ?? "")
    && ["send", "control", "mode", "edit"].includes(usage.group ?? "")
    && typeof usage.shortcut === "string" && /^[a-z0-9-]{1,64}$/.test(usage.shortcut)
    && Number.isSafeInteger(usage.count) && (usage.count ?? 0) >= 0 && (usage.count ?? 0) <= MAX_COUNT;
}

export function loadAgentKeyboardUsage(): Promise<boolean> {
  const requestGeneration = ++generation;
  return (async () => {
    const response = await fetch("/api/agent-keyboard/usage", {
      credentials: "same-origin", cache: "no-store", signal: AbortSignal.timeout(10000),
    });
    if (!response.ok) throw new Error(`Agent keyboard usage: ${response.status}`);
    const snapshot: unknown = await response.json();
    if (!snapshot || typeof snapshot !== "object") throw new Error("Invalid Agent keyboard usage response");
    const { userId: nextUserId, usage: records } = snapshot as { userId?: unknown; usage?: unknown };
    if (typeof nextUserId !== "string" || !nextUserId || nextUserId.length > 256
      || !Array.isArray(records) || records.length > 512) throw new Error("Invalid Agent keyboard usage response");
    if (requestGeneration !== generation) return false;
    if (userId !== undefined && userId !== nextUserId) counts.clear();
    userId = nextUserId;
    for (const record of records) {
      if (validUsage(record)) {
        const key = usageKey(record.agent, record.group, record.shortcut);
        counts.set(key, Math.max(counts.get(key) ?? 0, record.count));
      }
    }
    return true;
  })().catch(() => false);
}

export function recordAgentKeyboardUse(agent: MobileKeyboardAgent, group: string, value: string): void {
  const key = usageKey(agent, group, value);
  const requestGeneration = generation;
  counts.set(key, Math.min((counts.get(key) ?? 0) + 1, MAX_COUNT));
  void fetch("/api/agent-keyboard/usage", {
    method: "POST", credentials: "same-origin", headers: { "content-type": "application/json" },
    body: JSON.stringify({ agent, group, shortcut: value }), signal: AbortSignal.timeout(10000),
  }).then(async (response) => {
    if (!response.ok) return;
    const record: unknown = await response.json();
    if (requestGeneration === generation && validUsage(record) && usageKey(record.agent, record.group, record.shortcut) === key) {
      counts.set(key, Math.max(counts.get(key) ?? 0, record.count));
    }
  }).catch(() => { /* Input must work while offline. */ });
}

export function rankAgentKeyboardKeys(
  agent: MobileKeyboardAgent,
  group: string,
  keys: readonly MobileAgentKeyboardEntry[],
): MobileAgentKeyboardEntry[] {
  return [...keys].sort((left, right) =>
    (counts.get(usageKey(agent, group, right.key.value)) ?? 0) - (counts.get(usageKey(agent, group, left.key.value)) ?? 0),
  );
}
