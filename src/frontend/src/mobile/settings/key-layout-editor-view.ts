import { escapeAttr, escapeHtml } from "../../utils.ts";
import { mobileKeyHint, type MobileKeyTranslate } from "../key-hints.ts";
import type { MobileKeyboardKey } from "../keyboard-layout-types.ts";

export function renderKeyboardEditorKeys(keys: MobileKeyboardKey[], selectedId: string, tr: MobileKeyTranslate): string {
  return keys.map((key) => `
    <div class="mobile-keyboard-key-row${key.hidden ? " is-hidden" : ""}" data-mobile-layout-key="${escapeAttr(key.id)}" data-mobile-key-width="${key.width}" draggable="true">
      <button type="button" class="mobile-keyboard-key-card" data-mobile-key-select="${escapeAttr(key.id)}" aria-pressed="${key.id === selectedId}" aria-label="${escapeAttr([key.ariaLabel || key.label || key.value, mobileKeyHint(key, tr), key.hidden ? tr("mobileKeyboard.hidden") : ""].filter(Boolean).join(": "))}">
        ${key.icon ? `<i data-lucide="${escapeAttr(key.icon)}" aria-hidden="true"></i>` : ""}<span>${escapeHtml(key.label || key.ariaLabel || key.value)}</span>${key.hidden ? '<i data-lucide="eye-off" aria-hidden="true"></i>' : ""}
      </button>
    </div>
  `).join("");
}

export function renderKeyboardKeyInspector(keys: MobileKeyboardKey[], selectedId: string, tr: MobileKeyTranslate): string {
  const index = keys.findIndex((key) => key.id === selectedId);
  const key = keys[index];
  if (!key) return "";
  const id = escapeAttr(key.id);
  return `
    <div class="mobile-keyboard-inspector-heading"><strong>${escapeHtml(key.label || key.ariaLabel || key.value)}</strong><span>${escapeHtml(mobileKeyHint(key, tr))}</span></div>
    <div class="mobile-keyboard-key-controls">
      <label class="mobile-keyboard-key-width-control"><span>${escapeHtml(tr("field.mobileKeyboardKeyWidth"))}</span><select data-mobile-key-width="${id}">
        ${(["sm", "md", "lg"] as const).map((width, position) => `<option value="${width}"${key.width === width ? " selected" : ""}>${escapeHtml(tr(["option.mobileKeyboardWidthSmall", "option.mobileKeyboardWidthMedium", "option.mobileKeyboardWidthLarge"][position] as Parameters<MobileKeyTranslate>[0]))}</option>`).join("")}
      </select></label>
      <button type="button" class="icon-button" data-mobile-key-visibility="${id}" aria-pressed="${!key.hidden}" aria-label="${escapeAttr(tr(key.hidden ? "action.show" : "action.hide"))}"><i data-lucide="${key.hidden ? "eye-off" : "eye"}" aria-hidden="true"></i></button>
      <button type="button" class="icon-button" data-mobile-key-move="${id}" data-direction="-1" aria-label="${escapeAttr(tr("action.moveUp"))}"${index === 0 ? " disabled" : ""}><i data-lucide="chevron-left" aria-hidden="true"></i></button>
      <button type="button" class="icon-button" data-mobile-key-move="${id}" data-direction="1" aria-label="${escapeAttr(tr("action.moveDown"))}"${index === keys.length - 1 ? " disabled" : ""}><i data-lucide="chevron-right" aria-hidden="true"></i></button>
      ${key.custom ? `<button type="button" class="icon-button" data-mobile-key-edit="${id}" aria-label="${escapeAttr(tr("action.mobileKeyboardEditKey"))}"><i data-lucide="pencil" aria-hidden="true"></i></button><button type="button" class="icon-button danger" data-mobile-key-remove="${id}" aria-label="${escapeAttr(tr("action.remove"))}"><i data-lucide="trash-2" aria-hidden="true"></i></button>` : ""}
    </div>`;
}
