export function renderMobileKeyboardLayoutSettingsView(): string {
  return `
    <div class="settings-group mobile-keyboard-layout-settings" id="mobileKeyboardLayoutSettings">
      <div class="settings-group-title" data-i18n="section.mobileKeyboardLayout">Shortcut layout</div>
      <p class="settings-help" data-i18n="setting.mobileKeyboardLayoutHelp">Choose a layout for your terminal or editor. Keys follow the active program's bindings; presets do not change its editing mode.</p>
      <div class="mobile-keyboard-layout-toolbar">
        <label class="field">
          <span data-i18n="field.mobileKeyboardPreset">Preset</span>
          <select data-mobile-layout-preset>
            <option value="default" data-i18n="option.mobileKeyboardDefault">Default</option>
            <option value="operations" data-i18n="option.mobileKeyboardOperations">Operations</option>
            <option value="editor" data-i18n="option.mobileKeyboardEditor">Editor</option>
            <option value="vim">Vim / Neovim</option>
            <option value="emacs">Emacs</option>
            <option value="custom" data-i18n="option.mobileKeyboardCustom">Custom</option>
          </select>
        </label>
      </div>
      <details class="mobile-keyboard-customize" data-mobile-layout-customize>
        <summary data-i18n="mobileKeyboard.customize">Customize keys</summary>
        <div class="mobile-keyboard-customize-body">
        <div class="field mobile-keyboard-page-field">
          <span data-i18n="field.mobileKeyboardPage">Shortcut category</span>
          <div class="mobile-keyboard-editor-tabs" role="tablist" data-mobile-layout-page-tabs>
            <button type="button" role="tab" id="mobileLayoutPageTabMain" aria-controls="mobileKeyboardLayoutPagePanel" data-mobile-layout-page-tab="main" aria-selected="true" data-i18n="mobileKeyboard.pageMain">Keys</button>
            <button type="button" role="tab" id="mobileLayoutPageTabNav" aria-controls="mobileKeyboardLayoutPagePanel" data-mobile-layout-page-tab="nav" aria-selected="false" data-i18n="mobileKeyboard.pageNav">Nav</button>
            <button type="button" role="tab" id="mobileLayoutPageTabFn" aria-controls="mobileKeyboardLayoutPagePanel" data-mobile-layout-page-tab="fn" aria-selected="false">Fn</button>
            <button type="button" role="tab" id="mobileLayoutPageTabSym" aria-controls="mobileKeyboardLayoutPagePanel" data-mobile-layout-page-tab="sym" aria-selected="false" data-i18n="mobileKeyboard.pageSym">Symbols</button>
            <button type="button" role="tab" id="mobileLayoutPageTabOps" aria-controls="mobileKeyboardLayoutPagePanel" data-mobile-layout-page-tab="ops" aria-selected="false" data-i18n="mobileKeyboard.pageOps">Tools</button>
          </div>
        </div>
      <p class="settings-help settings-help-inline" data-i18n="setting.mobileKeyboardDragHelp">Drag a key to place it. Use the arrow buttons when you prefer precise keyboard control.</p>
      <div class="mobile-keyboard-key-list" id="mobileKeyboardLayoutPagePanel" role="tabpanel" aria-labelledby="mobileLayoutPageTabMain" data-mobile-layout-key-list></div>
      <div class="mobile-keyboard-key-inspector" data-mobile-layout-key-inspector></div>
      <details class="mobile-keyboard-custom-key">
        <summary data-mobile-key-editor-title data-i18n="action.mobileKeyboardAddKey">Add key</summary>
        <div class="mobile-keyboard-custom-key-fields">
          <label class="field"><span data-i18n="field.mobileKeyboardKeyType">Key type</span><select data-mobile-key-kind><option value="text" data-i18n="option.mobileKeyboardKeyText">Text</option><option value="shortcut" data-i18n="option.mobileKeyboardKeySpecial">Special key</option><option value="action" data-i18n="option.mobileKeyboardKeyAction">App action</option><option value="chord" data-i18n="option.mobileKeyboardKeyChord">Key combination</option></select></label>
          <label class="field"><span data-i18n="field.mobileKeyboardKeyLabel">Label</span><input type="text" maxlength="24" data-mobile-key-label /></label>
          <label class="field" data-mobile-key-text-field><span data-i18n="field.mobileKeyboardKeyText">Text or escape sequence</span><textarea rows="2" maxlength="1536" data-mobile-key-text></textarea><small data-i18n="setting.mobileKeyboardEscapeHelp">Use \\e or \\x1b for Escape; \\r, \\n and \\t are supported.</small></label>
          <label class="field" data-mobile-key-value-field hidden><span data-i18n="field.mobileKeyboardKeyValue">Key or action</span><select data-mobile-key-value></select></label>
          <label class="field"><span data-i18n="field.mobileKeyboardKeyWidth">Width</span><select data-mobile-key-new-width><option value="sm" data-i18n="option.mobileKeyboardWidthSmall">Narrow</option><option value="md" selected data-i18n="option.mobileKeyboardWidthMedium">Standard</option><option value="lg" data-i18n="option.mobileKeyboardWidthLarge">Wide</option></select></label>
          <label class="check-line" data-mobile-key-enter-field><input type="checkbox" data-mobile-key-enter /><span data-i18n="field.mobileKeyboardKeyEnter">Press Enter after sending this key's text</span></label>
          <p class="settings-help settings-help-inline" data-mobile-key-enter-help data-i18n="setting.mobileKeyboardKeyEnterHelp">Useful for commands that should run immediately.</p>
          <button class="command-button primary" type="button" data-mobile-key-add data-i18n="action.mobileKeyboardAddKey">Add key</button>
          <button class="command-button" type="button" data-mobile-key-cancel hidden data-i18n="action.cancel">Cancel</button>
          <p class="field-status" data-mobile-layout-status aria-live="polite"></p>
        </div>
      </details>
      <button class="command-button" type="button" data-mobile-layout-reset data-i18n="action.mobileKeyboardRestoreDefault">Restore default</button>
        </div>
      </details>
      <button class="command-button" type="button" data-mobile-layout-undo disabled data-i18n="action.mobileKeyboardUndo">Undo last layout change</button>
    </div>
  `;
}
