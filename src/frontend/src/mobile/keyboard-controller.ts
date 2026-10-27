import { createMobileKeyGestureController } from "./key-gesture-controller";
import type { MessageKey } from "../i18n";
import {
  mobileActionEventPhase,
  mobileActionRestoresKeyboard,
  mobileSyntheticActivation,
} from "./action-event-phase";
import {
  clearMobileSticky,
  createMobileStickyState,
  encodeMobileShortcutInput,
  isMobileModifierShortcut,
  mobileChordInput,
  toggleMobileModifier,
  transformMobileStickyInput,
} from "./shortcuts";
import {
  normalizeMobileQuickPhrases,
  renderMobileQuickPhraseKeyboardPanel,
} from "./quick-input";
import type { MobileQuickPhrase } from "../types";
import { updateSystemKeyboardToggleState } from "./system-keyboard-state";
import type { MobileKeyboardLayout, MobileKeyboardPresetId } from "./keyboard-layout-types.ts";
import { renderMobileKeyboardPanels } from "./keyboard-layout-view.ts";
import { renderMobileTypingKeyboard } from "./virtual-keyboard.ts";
import type { MobileAgentKeyboardContext } from "./agent-keyboard-context.ts";
import { renderMobileAgentKeyboard } from "./agent-keyboard-view.ts";
import { mobileAgentKeyboardGroups, mobileAgentName } from "./agent-keymap.ts";
import { rankAgentKeyboardKeys, recordAgentKeyboardUse } from "./agent-keyboard-usage.ts";

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

export type MobileShortcutRunOptions = {
  keepModifiers?: boolean;
};

export type MobileKeyboardControllerOptions = {
  root: HTMLElement;
  agentContext?: () => MobileAgentKeyboardContext | undefined;
  preserveSystemKeyboardState: () => () => void;
  onKeyInput: (data: string) => void;
  onPasteShortcut: () => Promise<void>;
  onAction: (action: string) => Promise<void>;
  onPhrase: (id: string) => Promise<void>;
  onLayoutChange: () => void;
  tr: Translate;
};

export type MobileKeyboardRenderInput = {
  phrases: MobileQuickPhrase[];
  agentContext?: MobileAgentKeyboardContext;
  tr: Translate;
  layout: MobileKeyboardLayout;
  preset: MobileKeyboardPresetId;
};

export function createMobileKeyboardController(options: MobileKeyboardControllerOptions) {
  const sticky = createMobileStickyState();
  let gestures: ReturnType<typeof createMobileKeyGestureController<HTMLButtonElement>> | undefined;
  let disposed = false;
  let deferredActionTimer: number | undefined;
  let expanded = false;
  let lastInput: MobileKeyboardRenderInput | undefined;
  let renderedContext: MobileAgentKeyboardContext | undefined;
  const keyboardRestores = new WeakMap<HTMLButtonElement, () => void>();

  function currentContext() {
    return options.agentContext ? options.agentContext() : lastInput?.agentContext;
  }

  function captureKeyboardState(button: HTMLButtonElement): () => void {
    const restore = options.preserveSystemKeyboardState();
    keyboardRestores.set(button, restore);
    return restore;
  }

  function takeKeyboardRestore(button: HTMLButtonElement): () => void {
    const restore = keyboardRestores.get(button) ?? options.preserveSystemKeyboardState();
    keyboardRestores.delete(button);
    return restore;
  }

  function clearDeferredAction() {
    window.clearTimeout(deferredActionTimer);
    deferredActionTimer = undefined;
  }

  function activateButton(button: HTMLButtonElement, repeating: boolean) {
    if (button.disabled || !options.root.contains(button)) return;
    if (renderedContext?.key !== currentContext()?.key) {
      if (lastInput) renderQuickInput(lastInput);
      return;
    }
    const activation = mobileSyntheticActivation(button, 0);
    if (!activation) return;
    const restore = takeKeyboardRestore(button);
    if (activation.kind === "shortcut" && button.closest(".mobile-agent-keyboard")) {
      const data = encodeMobileShortcutInput(activation.value, createMobileStickyState());
      if (data) {
        options.onKeyInput(data);
        if (renderedContext) recordAgentKeyboardUse(renderedContext.agent, button.closest<HTMLElement>("[data-agent-group]")?.dataset.agentGroup ?? "", activation.value);
      }
      clearSticky();
      restore();
    } else if (activation.kind === "shortcut") void runShortcut(activation.value, { keepModifiers: repeating }, restore);
    else if (activation.kind === "chord") {
      runChord(activation.value, restore);
      if (renderedContext && button.closest(".mobile-agent-keyboard")) recordAgentKeyboardUse(renderedContext.agent, button.closest<HTMLElement>("[data-agent-group]")?.dataset.agentGroup ?? "", activation.value);
    }
    else if (activation.kind === "page") activatePage(activation.value);
    else if (activation.kind === "phrase") void runPhrase(activation.value, restore);
    else if (activation.kind === "text") runText(activation.value, button.dataset.mobileAutoEnter === "true", restore);
    else if (activation.value === "pane-menu") {
      // Open the pane menu after the physical click has bubbled, matching its
      // existing dismissal boundary. System-keyboard actions never defer.
      clearDeferredAction();
      deferredActionTimer = window.setTimeout(() => {
        deferredActionTimer = undefined;
        if (!disposed && options.root.contains(button)) void runAction(activation.value, restore);
      }, 0);
    } else void runAction(activation.value, restore);
  }

  function bind() {
    if (disposed || gestures) return;
    gestures = createMobileKeyGestureController<HTMLButtonElement>({
      root: options.root, globalTarget: window, visibilityTarget: document,
      button: (target) => {
        const button = target instanceof Element ? target.closest<HTMLButtonElement>("button") : null;
        return button && options.root.contains(button) && mobileSyntheticActivation(button, 0) ? button : undefined;
      },
      usable: (button) => options.root.contains(button) && !button.disabled && button.isConnected
        && button.getClientRects().length > 0,
      inside: (button, x, y) => {
        const rect = button.getBoundingClientRect();
        return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      },
      activateOnClick: (button) => button.dataset.mobileAction !== "pane-menu" && mobileActionEventPhase(button.dataset.mobileAction ?? "") === "click",
      repeat: (button) => button.dataset.mobileRepeat === "true" && Boolean(button.dataset.mobileShortcut),
      start: (button) => { clearDeferredAction(); captureKeyboardState(button); },
      activate: activateButton,
      finish: (button, repeated) => { keyboardRestores.delete(button); if (repeated) clearSticky(); },
      capture: (button, id) => button.setPointerCapture?.(id),
      release: (button, id) => { if (button.hasPointerCapture?.(id)) button.releasePointerCapture(id); },
      setTimer: (callback, delay) => window.setTimeout(callback, delay),
      clearTimer: (timer) => window.clearTimeout(timer),
    });
    updateShortcutState();
  }

  function renderQuickInput(input: MobileKeyboardRenderInput): MobileQuickPhrase[] {
    stopRepeatInput();
    clearDeferredAction();
    const controls = options.root.querySelector<HTMLElement>(".mobile-keyboard-controls");
    if (!controls) return input.phrases;
    const focused = document.activeElement instanceof HTMLButtonElement && controls.contains(document.activeElement)
      ? document.activeElement : undefined;
    const focusAttribute = ["data-mobile-chord", "data-mobile-shortcut", "data-mobile-text", "data-mobile-action", "data-mobile-phrase"]
      .find((name) => focused?.hasAttribute(name));
    const focusValue = focusAttribute ? focused?.getAttribute(focusAttribute) : undefined;
    const focusPanel = focused?.closest<HTMLElement>("[data-mobile-panel]")?.dataset.mobilePanel;
    const scroll = new Map(Array.from(controls.querySelectorAll<HTMLElement>("[data-mobile-panel]"), (panel) => [panel.dataset.mobilePanel, panel.scrollTop]));
    const agentGroupScroll = new Map(Array.from(controls.querySelectorAll<HTMLElement>("[data-agent-group]"), (group) => [group.dataset.agentGroup, group.querySelector<HTMLElement>(".mobile-agent-keyboard-keys")?.scrollLeft ?? 0]));
    lastInput = input;
    const nextContext = currentContext();
    const sameContext = nextContext?.key === renderedContext?.key;
    if (nextContext?.key !== renderedContext?.key) clearSticky();
    renderedContext = nextContext;
    const currentPage = activePage();
    controls.innerHTML = `${renderMobileKeyboardPanels(input.layout, currentPage, input.tr)}${renderMobileTypingKeyboard()}<div class="mobile-keyboard-panel" data-mobile-panel="phrases" hidden></div>`;
    updateShortcutState();
    const mainPanel = controls.querySelector<HTMLElement>("[data-mobile-panel='main']");
    if (mainPanel && renderedContext) {
      mainPanel.classList.add("mobile-agent-keyboard");
      mainPanel.innerHTML = renderMobileAgentKeyboard(renderedContext, input.tr);
    }
    const mainTab = options.root.querySelector<HTMLButtonElement>("[data-mobile-page='main']");
    if (mainTab) mainTab.textContent = renderedContext ? mobileAgentName(renderedContext.agent) : input.tr("mobileKeyboard.pageMain");
    const phrasePanel = controls.querySelector<HTMLElement>("[data-mobile-panel='phrases']");
    if (!phrasePanel) return input.phrases;

    const phrases = normalizeMobileQuickPhrases(input.phrases);

    phrasePanel.innerHTML = renderMobileQuickPhraseKeyboardPanel(phrases);
    activatePage(currentPage === "phrases" && !phrases.length ? "sym" : currentPage);
    syncExpandedView();
    if (sameContext) {
      controls.querySelectorAll<HTMLElement>("[data-mobile-panel]").forEach((panel) => { panel.scrollTop = scroll.get(panel.dataset.mobilePanel) ?? 0; });
      controls.querySelectorAll<HTMLElement>("[data-agent-group]").forEach((group) => {
        const keys = group.querySelector<HTMLElement>(".mobile-agent-keyboard-keys");
        if (keys) keys.scrollLeft = agentGroupScroll.get(group.dataset.agentGroup) ?? 0;
      });
      if (focusAttribute && focusValue != null && focusPanel) {
        controls.querySelector<HTMLButtonElement>(`[data-mobile-panel="${CSS.escape(focusPanel)}"] [${focusAttribute}="${CSS.escape(focusValue)}"]`)?.focus({ preventScroll: true });
      }
    }
    return phrases;
  }

  function activatePage(page: string) {
    if (!options.root.querySelector(`[data-mobile-panel="${CSS.escape(page)}"]`)) return;
    if (page === "main" && page !== activePage() && renderedContext) {
      for (const group of mobileAgentKeyboardGroups(renderedContext.agent)) {
        const row = options.root.querySelector<HTMLElement>(`[data-agent-group="${group.id}"] .mobile-agent-keyboard-keys`);
        if (!row) continue;
        const buttons = new Map(Array.from(row.querySelectorAll<HTMLButtonElement>("button"), (button) =>
          [button.dataset.mobileChord ?? button.dataset.mobileShortcut, button]));
        for (const entry of rankAgentKeyboardKeys(renderedContext.agent, group.id, group.keys)) {
          const button = buttons.get(entry.key.value);
          if (button) row.append(button);
        }
        row.scrollLeft = 0;
      }
    }
    if (page !== activePage()) stopRepeatInput();
    options.root.dataset.activePage = page;
    options.root.querySelectorAll<HTMLButtonElement>("[data-mobile-page]").forEach((button) => {
      const active = button.dataset.mobilePage === page;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
    options.root.querySelectorAll<HTMLElement>("[data-mobile-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.mobilePanel !== page;
    });
    if (expanded) options.onLayoutChange();
  }

  function activePage(): string {
    return options.root.dataset.activePage ?? "main";
  }

  function syncExpandedView() {
    options.root.dataset.expanded = String(expanded);
    const panel = options.root.querySelector<HTMLElement>(".mobile-expanded-keyboard");
    if (panel) panel.hidden = !expanded;
    const button = options.root.querySelector<HTMLButtonElement>("[data-mobile-action='toggle-expanded-keyboard']");
    if (button) {
      const label = options.tr(expanded ? "action.collapseMobileKeyboard" : "action.expandMobileKeyboard");
      button.setAttribute("aria-expanded", String(expanded));
      button.setAttribute("aria-label", label);
      button.title = label;
    }
    document.body.classList.toggle("mobile-keyboard-expanded", expanded);
  }

  function setExpanded(next: boolean, preserveModifiers = false) {
    if (expanded === next) return;
    stopRepeatInput();
    if (!preserveModifiers) clearSticky();
    expanded = next;
    syncExpandedView();
    options.onLayoutChange();
  }

  function toggleExpanded() {
    setExpanded(!expanded);
  }

  async function runShortcut(
    shortcut: string,
    runOptions: MobileShortcutRunOptions,
    restoreKeyboard: () => void,
  ) {
    if (isMobileModifierShortcut(shortcut)) {
      toggleMobileModifier(sticky, shortcut);
      updateShortcutState();
      restoreKeyboard();
      return;
    }

    if (shortcut === "paste") {
      await options.onPasteShortcut();
      if (disposed) return;
      clearSticky();
      restoreKeyboard();
      return;
    }

    const data = encodeMobileShortcutInput(shortcut, sticky);
    if (data) {
      options.onKeyInput(data);
    }
    if (!runOptions.keepModifiers) {
      clearSticky();
    }
    restoreKeyboard();
  }

  function runChord(chord: string, restoreKeyboard: () => void) {
    const data = mobileChordInput(chord);
    if (data) {
      options.onKeyInput(data);
    }
    clearSticky();
    restoreKeyboard();
  }

  function runText(text: string, autoEnter: boolean, restoreKeyboard: () => void) {
    if (text) options.onKeyInput(autoEnter ? `${text}\r` : text);
    clearSticky();
    restoreKeyboard();
  }

  async function runPhrase(id: string, restoreKeyboard: () => void) {
    await options.onPhrase(id);
    if (!disposed) restoreKeyboard();
  }

  async function runAction(action: string, restoreKeyboard: () => void) {
    await options.onAction(action);
    if (disposed) return;
    const systemKeyboardActive = action === "toggle-system-keyboard"
      && options.root.querySelector<HTMLButtonElement>("[data-mobile-action='toggle-system-keyboard']")?.getAttribute("aria-pressed") === "true";
    if (!systemKeyboardActive) clearSticky();
    if (mobileActionRestoresKeyboard(action)) restoreKeyboard();
  }

  function stopRepeatInput() {
    gestures?.cancel();
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    clearDeferredAction();
    gestures?.dispose();
    gestures = undefined;
    lastInput = undefined;
    renderedContext = undefined;
    document.body.classList.remove("mobile-keyboard-expanded");
  }

  function encodeStickyInput(text: string, source: string): string | undefined {
    const encoded = transformMobileStickyInput(sticky, text, source);
    if (encoded) updateShortcutState();
    return encoded;
  }

  function clearSticky() {
    clearMobileSticky(sticky);
    updateShortcutState();
  }

  function updateShortcutState() {
    options.root.querySelectorAll<HTMLButtonElement>("[data-mobile-modifier]").forEach((button) => {
      const modifier = button.dataset.mobileModifier;
      const active = modifier === "ctrl" || modifier === "alt" || modifier === "shift" ? sticky[modifier] : false;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  function updateSystemKeyboardState(enabled: boolean) {
    if (enabled) setExpanded(false, true);
    const button = options.root.querySelector<HTMLButtonElement>("[data-mobile-action='toggle-system-keyboard']");
    if (!button) return;
    updateSystemKeyboardToggleState(button, enabled);
  }

  return {
    bind,
    dispose,
    renderQuickInput,
    activatePage,
    stopRepeatInput,
    encodeStickyInput,
    clearSticky,
    updateShortcutState,
    updateSystemKeyboardState,
    isExpanded: () => expanded,
    setExpanded,
    toggleExpanded,
  };
}
