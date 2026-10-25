import { mobileKeyboardPresetLayout } from "./keyboard-layout.ts";
import { renderMobileKeyboardPanels } from "./keyboard-layout-view.ts";
import { renderMobileQuickBar } from "./quick-bar.ts";
import { renderMobileTypingKeyboard } from "./virtual-keyboard.ts";

export function renderMobileKeyboardView(): string {
  const layout = mobileKeyboardPresetLayout("default");
  return `
      <div class="mobile-shortcuts" id="mobileShortcuts" role="region" aria-label="Terminal shortcuts" data-i18n-aria="menu.mobileShortcuts" data-active-page="main" data-expanded="false">
        <div class="mobile-keyboard-toolbar">
          <div class="mobile-keyboard-quick-keys" role="toolbar" aria-label="Quick terminal keys" data-i18n-aria="label.mobileQuickKeys">
            ${renderMobileQuickBar()}
          </div>
          <div class="mobile-keyboard-toolbar-actions">
            <button type="button" class="mobile-system-keyboard-toggle" data-mobile-action="toggle-system-keyboard" aria-pressed="false" aria-label="Toggle system keyboard" title="Toggle system keyboard" data-i18n-aria="action.toggleSystemKeyboard" data-i18n-title="action.toggleSystemKeyboard"><i class="mobile-keyboard-hidden-icon" data-lucide="keyboard"></i><i class="mobile-keyboard-visible-icon" data-lucide="keyboard-off"></i></button>
            <button type="button" class="mobile-expand-keyboard-toggle" data-mobile-action="toggle-expanded-keyboard" aria-expanded="false" aria-controls="mobileExpandedKeyboard" aria-label="Expand keyboard" title="Expand keyboard" data-i18n-aria="action.expandMobileKeyboard" data-i18n-title="action.expandMobileKeyboard"><i class="mobile-keyboard-expand-icon" data-lucide="chevron-up" aria-hidden="true"></i><i class="mobile-keyboard-collapse-icon" data-lucide="chevron-down" aria-hidden="true"></i></button>
          </div>
        </div>
        <div class="mobile-expanded-keyboard" id="mobileExpandedKeyboard" hidden>
          <div class="mobile-keyboard-pages">
            <div class="mobile-keyboard-page-tabs" role="toolbar" aria-label="Terminal shortcut pages" data-i18n-aria="menu.mobileShortcuts">
              <button type="button" class="active" data-mobile-page="main" aria-pressed="true" data-i18n="mobileKeyboard.pageMain">Keys</button>
              <button type="button" data-mobile-page="nav" aria-pressed="false" data-i18n="mobileKeyboard.pageNav">Nav</button>
              <button type="button" data-mobile-page="fn" aria-pressed="false">Fn</button>
              <button type="button" data-mobile-page="sym" aria-pressed="false" data-i18n="mobileKeyboard.pageSym">Symbols</button>
              <button type="button" data-mobile-page="typing" aria-pressed="false">ABC</button>
              <button type="button" data-mobile-page="ops" aria-pressed="false" data-i18n="mobileKeyboard.pageOps">Tools</button>
            </div>
            <span class="mobile-shortcut-clock" id="mobileShortcutClock" role="timer" aria-label="Current time" data-i18n-aria="label.currentTime"></span>
          </div>
          <div class="mobile-keyboard-tool-actions" data-mobile-panel="ops" hidden>
            <button type="button" data-mobile-action="compose-text"><i data-lucide="message-square-text" aria-hidden="true"></i><span data-i18n="mobileComposer.title">Draft input</span></button>
            <button type="button" data-mobile-action="secret-file"><i data-lucide="file-key" aria-hidden="true"></i><span data-i18n="secret.title">Secret file</span></button>
            <button type="button" data-mobile-action="command-palette"><i data-lucide="list-filter" aria-hidden="true"></i><span data-i18n="mobilePalette.title">Phrases and keys</span></button>
          </div>
          <div class="mobile-keyboard-controls">
            ${renderMobileKeyboardPanels(layout)}
            ${renderMobileTypingKeyboard()}
            <div class="mobile-keyboard-panel" data-mobile-panel="phrases" hidden></div>
          </div>
        </div>
      </div>
  `;
}
