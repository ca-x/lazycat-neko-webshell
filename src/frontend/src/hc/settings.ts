export const DEFAULT_HC_HISTORY_LINES = 1000;
export const MAX_HC_HISTORY_LINES = 10000;

export function normalizeHcHistoryLines(value: unknown): number {
  const numeric = typeof value === "number" ? value
    : typeof value === "string" && value.trim() ? Number(value) : NaN;
  return Number.isFinite(numeric)
    ? Math.min(MAX_HC_HISTORY_LINES, Math.max(0, Math.trunc(numeric)))
    : DEFAULT_HC_HISTORY_LINES;
}
