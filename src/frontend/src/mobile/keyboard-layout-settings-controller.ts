import type { MessageKey } from "../i18n.ts";
import type { MobileKeyboardKeyWidth, MobileKeyboardLayout, MobileKeyboardPageId, MobileKeyboardPresetId } from "./keyboard-layout-types.ts";
import { escapeHtml } from "../utils.ts";
import {
  MOBILE_KEYBOARD_PAGE_IDS,
  mobileKeyboardPresetLayout,
  moveMobileKeyboardKey,
  moveMobileKeyboardKeyToIndex,
  removeMobileKeyboardKey,
  resolveMobileKeyboardLayout,
  updateMobileKeyboardKey,
} from "./keyboard-layout.ts";

import { createMobileKeyEditor } from "./settings/key-editor.ts";
import { installMobileKeyboardReorderGesture } from "./settings/keyboard-reorder-gesture.ts";
import { renderKeyboardEditorKeys, renderKeyboardKeyInspector } from "./settings/key-layout-editor-view.ts";

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

type Options = {
  root: HTMLElement;
  preset: () => MobileKeyboardPresetId;
  layout: () => MobileKeyboardLayout;
  setPreset: (preset: MobileKeyboardPresetId) => void;
  setLayout: (layout: MobileKeyboardLayout) => void;
  save: () => void;
  changed: () => void;
  updateIcons: () => void;
  tr: Translate;
};

export function createMobileKeyboardLayoutSettingsController(options: Options) {
  let pageId: MobileKeyboardPageId = "main";
  let selectedKeyId = "";
  const preset = options.root.querySelector<HTMLSelectElement>("[data-mobile-layout-preset]");
  const pageTabs = options.root.querySelector<HTMLElement>("[data-mobile-layout-page-tabs]");
  const list = options.root.querySelector<HTMLElement>("[data-mobile-layout-key-list]");
  const inspector = options.root.querySelector<HTMLElement>("[data-mobile-layout-key-inspector]");
  const undo = options.root.querySelector<HTMLButtonElement>("[data-mobile-layout-undo]");
  let previous: { preset: MobileKeyboardPresetId; layout: MobileKeyboardLayout } | undefined;
  let editor: ReturnType<typeof createMobileKeyEditor> | undefined;
  let reorderGesture: ReturnType<typeof installMobileKeyboardReorderGesture> | undefined;

  function remember() {
    previous = { preset: options.preset(), layout: structuredClone(options.layout()) };
  }

  function currentLayout() {
    return resolveMobileKeyboardLayout(options.preset(), options.layout());
  }

  function commit(layout: MobileKeyboardLayout) {
    const focused = document.activeElement instanceof HTMLElement && (list?.contains(document.activeElement) || inspector?.contains(document.activeElement)) ? document.activeElement : undefined;
    const focusAttributes = focused ? [...focused.attributes].filter((attr) => attr.name.startsWith("data-mobile-key-") || attr.name === "data-direction") : [];
    const keys = currentLayout().pages.find((page) => page.id === pageId)?.keys ?? [];
    const added = layout.pages.find((page) => page.id === pageId)?.keys.find((key) => !keys.some((existing) => existing.id === key.id));
    if (added) selectedKeyId = added.id;
    remember();
    options.setLayout(layout);
    options.setPreset("custom");
    options.save();
    options.changed();
    render();
    if (focusAttributes.length) {
      const selector = focusAttributes.map((attr) => `[${attr.name}="${CSS.escape(attr.value)}"]`).join("");
      const replacement = options.root.querySelector<HTMLElement>(selector);
      if (replacement && !replacement.matches(":disabled")) replacement.focus({ preventScroll: true });
      else undo?.focus({ preventScroll: true });
    }
  }

  function render() {
    reorderGesture?.cancel();
    if (undo) { undo.disabled = !previous; undo.hidden = !previous; }
    if (preset) preset.value = options.preset();
    pageTabs?.querySelectorAll<HTMLButtonElement>("[data-mobile-layout-page-tab]").forEach((tab) => {
      const selected = tab.dataset.mobileLayoutPageTab === pageId;
      tab.setAttribute("aria-selected", String(selected));
      tab.tabIndex = selected ? 0 : -1;
    });
    list?.setAttribute("aria-labelledby", `mobileLayoutPageTab${pageId.charAt(0).toUpperCase()}${pageId.slice(1)}`);
    if (!list) return;
    const keys = currentLayout().pages.find((item) => item.id === pageId)?.keys ?? [];
    if (!keys.some((key) => key.id === selectedKeyId)) selectedKeyId = keys[0]?.id ?? "";
    list.innerHTML = keys.length ? renderKeyboardEditorKeys(keys, selectedKeyId, options.tr) : `<p class="empty">${escapeHtml(options.tr("status.mobileKeyboardPageEmpty"))}</p>`;
    if (inspector) inspector.innerHTML = renderKeyboardKeyInspector(keys, selectedKeyId, options.tr);
    options.updateIcons();
  }

  function bind() {
    editor = createMobileKeyEditor({ root: options.root, layout: currentLayout, page: () => pageId, commit, tr: options.tr });
    undo?.addEventListener("click", () => {
      if (!previous) return;
      const snapshot = previous;
      previous = undefined;
      editor?.reset();
      options.setLayout(snapshot.layout);
      options.setPreset(snapshot.preset);
      options.save();
      options.changed();
      render();
    });
    preset?.addEventListener("change", () => {
      const value = preset.value as MobileKeyboardPresetId;
      if (!["default", "operations", "editor", "vim", "emacs", "custom"].includes(value)) return;
      remember();
      editor?.reset();
      options.setPreset(value);
      options.save();
      options.changed();
      render();
    });
    pageTabs?.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target.closest<HTMLButtonElement>("[data-mobile-layout-page-tab]") : null;
      const nextPage = target?.dataset.mobileLayoutPageTab as MobileKeyboardPageId | undefined;
      if (nextPage && MOBILE_KEYBOARD_PAGE_IDS.includes(nextPage)) {
        editor?.reset();
        pageId = nextPage;
        render();
      }
    });
    pageTabs?.addEventListener("keydown", (event) => {
      if (!(event.target instanceof HTMLButtonElement)) return;
      const tabs = [...pageTabs.querySelectorAll<HTMLButtonElement>("[data-mobile-layout-page-tab]")];
      const current = tabs.indexOf(event.target);
      if (current < 0 || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
      event.preventDefault();
      const next = tabs[(current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
      next?.click();
      next?.focus();
    });
    options.root.addEventListener("change", (event) => {
      const control = event.target instanceof Element ? event.target.closest<HTMLSelectElement>("[data-mobile-key-width]") : null;
      if (!control) return;
      commit(updateMobileKeyboardKey(currentLayout(), pageId, control.dataset.mobileKeyWidth ?? "", { width: control.value as MobileKeyboardKeyWidth }));
    });
    options.root.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const select = target?.closest<HTMLButtonElement>("[data-mobile-key-select]");
      if (select) {
        selectedKeyId = select.dataset.mobileKeySelect ?? "";
        render();
        list?.querySelector<HTMLButtonElement>(`[data-mobile-key-select="${CSS.escape(selectedKeyId)}"]`)?.focus({ preventScroll: true });
        return;
      }
      const move = target?.closest<HTMLButtonElement>("[data-mobile-key-move]");
      if (move) return commit(moveMobileKeyboardKey(currentLayout(), pageId, move.dataset.mobileKeyMove ?? "", move.dataset.direction === "-1" ? -1 : 1));
      const visibility = target?.closest<HTMLButtonElement>("[data-mobile-key-visibility]");
      if (visibility) {
        const id = visibility.dataset.mobileKeyVisibility ?? "";
        const key = currentLayout().pages.find((item) => item.id === pageId)?.keys.find((item) => item.id === id);
        if (key) commit(updateMobileKeyboardKey(currentLayout(), pageId, id, { hidden: !key.hidden }));
        return;
      }
      const edit = target?.closest<HTMLButtonElement>("[data-mobile-key-edit]");
      if (edit) {
        const key = currentLayout().pages.find((item) => item.id === pageId)?.keys.find((item) => item.id === edit.dataset.mobileKeyEdit);
        if (key) editor?.edit(key);
        return;
      }
      const remove = target?.closest<HTMLButtonElement>("[data-mobile-key-remove]");
      if (remove) return commit(removeMobileKeyboardKey(currentLayout(), pageId, remove.dataset.mobileKeyRemove ?? ""));
      if (target?.closest("[data-mobile-layout-reset]")) {
        remember();
        editor?.reset();
        options.setLayout(mobileKeyboardPresetLayout("default"));
        options.setPreset("default");
        options.save();
        options.changed();
        render();
        return;
      }

    });
    options.root.addEventListener("dragstart", (event) => {
      const row = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mobile-layout-key]") : null;
      if (!row) return;
      row.classList.add("is-dragging");
      event.dataTransfer?.setData("text/plain", row.dataset.mobileLayoutKey ?? "");
      if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    });
    options.root.addEventListener("dragend", (event) => {
      const row = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mobile-layout-key]") : null;
      row?.classList.remove("is-dragging");
      list?.querySelectorAll(".is-drag-over").forEach((item) => item.classList.remove("is-drag-over"));
    });
    options.root.addEventListener("dragover", (event) => {
      const row = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mobile-layout-key]") : null;
      if (!row) return;
      event.preventDefault();
      list?.querySelectorAll(".is-drag-over").forEach((item) => item.classList.remove("is-drag-over"));
      row.classList.add("is-drag-over");
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    });
    options.root.addEventListener("drop", (event) => {
      const row = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-mobile-layout-key]") : null;
      if (!row) return;
      event.preventDefault();
      const sourceId = event.dataTransfer?.getData("text/plain") ?? "";
      const targetId = row.dataset.mobileLayoutKey ?? "";
      const keys = currentLayout().pages.find((item) => item.id === pageId)?.keys ?? [];
      const targetIndex = keys.findIndex((item) => item.id === targetId);
      const insertIndex = targetIndex;
      if (sourceId && targetIndex >= 0) commit(moveMobileKeyboardKeyToIndex(currentLayout(), pageId, sourceId, insertIndex));
    });
    if (list) reorderGesture = installMobileKeyboardReorderGesture({
      root: options.root,
      list,
      scrollHost: list.closest<HTMLElement>(".settings-panels"),
      move: (sourceId, targetId) => {
        const keys = currentLayout().pages.find((item) => item.id === pageId)?.keys ?? [];
        const targetIndex = keys.findIndex((item) => item.id === targetId);
        if (targetIndex >= 0) commit(moveMobileKeyboardKeyToIndex(currentLayout(), pageId, sourceId, targetIndex));
      },
    });
    render();
  }

  return { bind, render };
}
