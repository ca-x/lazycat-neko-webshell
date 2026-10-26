import { escapeAttr, escapeHtml } from "../utils.ts";
import { mobileAgentKeyboardGroups, mobileAgentName } from "./agent-keymap.ts";
import type { MobileAgentKeyboardContext } from "./agent-keyboard-context.ts";
import { renderMobileKeyboardKey } from "./keyboard-layout-view.ts";
import type { MobileKeyTranslate } from "./key-hints.ts";

export function renderMobileAgentKeyboard(context: MobileAgentKeyboardContext, tr: MobileKeyTranslate): string {
  return `<div class="mobile-agent-keyboard-heading">${escapeHtml(mobileAgentName(context.agent))}</div>
    ${mobileAgentKeyboardGroups(context.agent).map((group) => `<section class="mobile-agent-keyboard-group" aria-label="${escapeAttr(tr(group.label))}">
      <h3>${escapeHtml(tr(group.label))}</h3>
      ${group.keys.map((entry) => renderMobileKeyboardKey(entry.key, tr, {
        label: tr(entry.label),
        hint: [entry.key.label, entry.condition ? tr(entry.condition) : ""].filter(Boolean).join(" · "),
      })).join("")}
    </section>`).join("")}`;
}
