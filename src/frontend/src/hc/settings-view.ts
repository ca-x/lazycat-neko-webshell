import type { Settings, TerminalPane } from "../types.ts";
import { MAX_HC_HISTORY_LINES } from "./settings.ts";

type HcSettingsElements = {
  hcHistorySettings: HTMLElement;
  hcHistoryLines: HTMLInputElement;
};

export function renderHcHistorySettings(): string {
  return `<div id="hcHistorySettings" hidden>
    <label class="field">
      <span data-i18n="field.hcHistoryLines">HC history lines</span>
      <input id="hcHistoryLines" type="number" min="0" max="${MAX_HC_HISTORY_LINES}" step="1" aria-describedby="hcHistoryLinesHelp" />
    </label>
    <p id="hcHistoryLinesHelp" class="settings-help" data-i18n="setting.hcHistoryLinesHelp">History lines requested when attaching to HC (0–10000; default 1000). Applies on the next attach; 0 requests no scrollback.</p>
  </div>`;
}

export function syncHcHistorySettings(elements: HcSettingsElements, settings: Pick<Settings, "hcHistoryLines">, available: boolean): void {
  elements.hcHistorySettings.hidden = !available;
  elements.hcHistoryLines.value = String(settings.hcHistoryLines);
}

export function hcHistorySettingsVisible(
  available: boolean,
  panes: readonly Pick<TerminalPane, "sessionBackend" | "selector">[],
  selector: string,
): boolean {
  return available || panes.some((pane) => pane.sessionBackend === "hc" && pane.selector.trim() === selector.trim());
}
