import type { MobileKeyboardKey, MobileKeyboardLayout } from "./keyboard-layout-types.ts";
import { escapeAttr, escapeHtml } from "../utils.ts";
import { mobileKeyHint, type MobileKeyTranslate } from "./key-hints.ts";

export function renderMobileKeyboardPanels(layout: MobileKeyboardLayout, activePage = "main", tr?: MobileKeyTranslate): string {
  return layout.pages.map((page) => `
    <div class="mobile-keyboard-panel" data-mobile-panel="${escapeAttr(page.id)}"${page.id === activePage ? "" : " hidden"}>
      ${page.keys.filter((key) => !key.hidden).map((key) => renderMobileKeyboardKey(key, tr)).join("")}
    </div>
  `).join("");
}

export function renderMobileKeyboardKey(key: MobileKeyboardKey, tr?: MobileKeyTranslate): string {
  const data = key.kind === "shortcut"
    ? `data-mobile-shortcut="${escapeAttr(key.value)}"${key.repeat ? ' data-mobile-repeat="true"' : ""}${isModifier(key.value) ? ` data-mobile-modifier="${escapeAttr(key.value)}" aria-pressed="false"` : ""}`
    : key.kind === "chord"
      ? `data-mobile-chord="${escapeAttr(key.value)}"`
      : key.kind === "action"
        ? `data-mobile-action="${escapeAttr(key.value)}"`
        : `data-mobile-text="${escapeTerminalDataAttribute(key.value)}"${key.autoEnter ? ' data-mobile-auto-enter="true"' : ""}`;
  const content = key.icon
    ? `<i data-lucide="${escapeAttr(key.icon)}"></i>${key.label ? `<span>${escapeHtml(key.label)}</span>` : ""}`
    : escapeHtml(key.label || key.value);
  const hint = mobileKeyHint(key, tr);
  return `<button type="button" ${data} data-mobile-key-width="${escapeAttr(key.width)}" aria-label="${escapeAttr([key.ariaLabel || key.label || key.value, hint].filter(Boolean).join(": "))}"><span>${content}</span>${hint ? `<small>${escapeHtml(hint)}</small>` : ""}</button>`;
}

function isModifier(value: string): boolean {
  return value === "ctrl" || value === "alt" || value === "shift";
}

function escapeTerminalDataAttribute(value: string): string {
  return escapeAttr(value).replace(/[\u0000-\u001f\u007f]/g, (character) => `&#${character.charCodeAt(0)};`);
}
