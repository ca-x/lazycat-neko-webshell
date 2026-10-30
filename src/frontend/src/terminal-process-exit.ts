import type { MessageKey } from "./i18n";
import type { TerminalServerEvent } from "./terminal-protocol.ts";
import type { TerminalPane } from "./types.ts";

type ProcessExitEvent = Extract<TerminalServerEvent, { type: "process-exit" }>;
type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

export function observeTerminalProcessExit(
  pane: Pick<TerminalPane, "processExitObserved" | "exited" | "sessionStatus" | "transport">,
  event: ProcessExitEvent,
  tr: Translate,
): string {
  pane.processExitObserved = true;
  pane.exited = true;
  pane.sessionStatus = "exited";
  const code = typeof event.exit_code === "number" && Number.isInteger(event.exit_code)
    ? event.exit_code : undefined;
  // Restty requires a number. -1 means unknown, never a successful process exit.
  pane.transport?.notifyExit(code ?? -1);
  if (event.message) return event.message;
  if (code !== undefined) return tr("status.processExited", { code });
  const message = event.signal !== undefined && event.signal !== null
    ? tr("status.processTerminatedSignal", { signal: event.signal })
    : tr("status.processTerminated");
  return event.reason ? `${message} (${event.reason})` : message;
}
