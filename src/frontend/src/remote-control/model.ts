import type { TerminalTab } from "../types";
import type { RemoteState } from "./api.ts";

export type SessionChoice = { id: string; name: string; selector: string; backend: string };
export function remoteSessionChoices(tabs: TerminalTab[]): SessionChoice[] {
  const choices = new Map<string, SessionChoice>();
  for (const tab of tabs) for (const pane of tab.panes) {
    if (!pane.sessionId || pane.sessionStatus === "closed" || pane.status === "closed") continue;
    if (!pane.selector || pane.selector.startsWith("client:") || pane.selector.startsWith("microvm:")) continue;
    choices.set(pane.sessionId, { id: pane.sessionId, name: pane.label || tab.label, selector: pane.selector, backend: pane.sessionBackend });
  }
  return [...choices.values()];
}
export function remoteStatus(state: RemoteState): "disabled" | "online" | "connecting" | "reconnecting" | "offline" {
  if (!state.enabled) return "disabled";
  return ["online", "connecting", "reconnecting"].includes(state.status)
    ? state.status as "online" | "connecting" | "reconnecting" : "offline";
}
export function freshRtt(value: number | null, online: boolean, receivedAt: number, now: number): number | null {
  return online && now - receivedAt < 30000 && value !== null && Number.isFinite(value) && value >= 0 ? value : null;
}

export function unknownOperationCount(state: RemoteState): number {
  const records = [...(state.operations?.active ?? []), ...(state.operations?.recent ?? [])];
  return new Set(records.filter(record => record.outcomeUnknown).map(record => record.id)).size;
}
