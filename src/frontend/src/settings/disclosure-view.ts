import type { MessageKey } from "../i18n";
import { escapeAttr, escapeHtml } from "../utils";

type SettingsDisclosure = {
  id: string;
  title: MessageKey;
  description: MessageKey;
  content: string;
  hidden?: boolean;
  open?: boolean;
  tr?: (key: MessageKey) => string;
};

/** Native disclosure preserves keyboard semantics and the form's mounted state. */
export function renderSettingsDisclosure(group: SettingsDisclosure): string {
  return `<details class="settings-disclosure" id="${escapeAttr(group.id)}" ${group.hidden ? "hidden" : ""} ${group.open ? "open" : ""}>
    <summary>
      <span class="settings-disclosure-label">
        <span class="settings-disclosure-title" data-i18n="${group.title}">${escapeHtml(group.tr?.(group.title) ?? "")}</span>
        <span class="settings-disclosure-description" data-i18n="${group.description}">${escapeHtml(group.tr?.(group.description) ?? "")}</span>
      </span>
      <i data-lucide="chevron-down" aria-hidden="true"></i>
    </summary>
    <div class="settings-disclosure-body">${group.content}</div>
  </details>`;
}
