import { normalizeHcHistoryLines } from "./settings.ts";
import type { TerminalPane } from "../types.ts";
import type { WebshellSocketUrlOptions } from "../webshell-backend.ts";

// HC sequences belong to one core process. Every attach requests a replacement
// screen, never a native history cursor or a command restart.
export function hcAttachOptions(options: WebshellSocketUrlOptions): WebshellSocketUrlOptions {
  if (options.sessionBackend !== "hc") return options;
  return { ...options, after: 0, restart: false, terminalReplyAuthority: "server", controlMode: undefined,
    hcHistoryLines: normalizeHcHistoryLines(options.hcHistoryLines) };
}

export function resetHcTerminalForReplay(pane: TerminalPane): void {
  if (pane.sessionBackend !== "hc") return;
  // The replay controller has locked input and the renderer already suppresses
  // generated replies, including probes parsed across slow repaint frames.
  pane.term?.reset();
  pane.decoder = new TextDecoder();
  pane.lastOutputSequence = 0;
  pane.lastReplayAfter = 0;
  pane.titleBuffer = "";
}
