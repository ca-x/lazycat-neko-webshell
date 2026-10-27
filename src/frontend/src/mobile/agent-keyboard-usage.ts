import type { MobileAgentKeyboardEntry, MobileKeyboardAgent } from "./agent-keymap.ts";

const STORAGE_KEY = "mobile-agent-keyboard-usage-v1";
const MAX_COUNT = 1_000_000;
type Counts = Record<string, number>;

function readCounts(): Counts {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY) ?? "{}";
    if (stored.length > 32_768) return {};
    const value: unknown = JSON.parse(stored);
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).slice(0, 512).filter(([key, count]) =>
      /^[a-z]+:[a-z]+:[a-z0-9-]+$/.test(key) && Number.isSafeInteger(count) && (count as number) > 0 && (count as number) <= MAX_COUNT,
    )) as Counts;
  } catch {
    return {};
  }
}

function usageKey(agent: MobileKeyboardAgent, group: string, value: string): string {
  return `${agent}:${group}:${value}`;
}

export function recordAgentKeyboardUse(agent: MobileKeyboardAgent, group: string, value: string): void {
  try {
    const counts = readCounts();
    const key = usageKey(agent, group, value);
    counts[key] = Math.min((counts[key] ?? 0) + 1, MAX_COUNT);
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(counts));
  } catch {
    // Private browsing or storage restrictions must never block terminal input.
  }
}

export function rankAgentKeyboardKeys(
  agent: MobileKeyboardAgent,
  group: string,
  keys: readonly MobileAgentKeyboardEntry[],
): MobileAgentKeyboardEntry[] {
  const counts = readCounts();
  return [...keys].sort((left, right) =>
    (counts[usageKey(agent, group, right.key.value)] ?? 0) - (counts[usageKey(agent, group, left.key.value)] ?? 0),
  );
}
