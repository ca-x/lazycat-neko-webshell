import { renderSettingsSectionIntro } from "./navigation-view";
import { renderSettingsDisclosure } from "./disclosure-view";

export function renderFontsSettingsView(): string {
  return `
            <section class="settings-section" id="fontSettingsRootPanel" data-settings-panel="fonts" role="tabpanel" aria-labelledby="settings-tab-fonts" hidden>
              ${renderSettingsSectionIntro("fonts")}
              <div class="settings-tabs settings-sub-tabs" id="fontTabs" role="tablist" aria-label="Fonts" data-i18n-aria="section.fonts">
                <button type="button" role="tab" aria-selected="true" aria-controls="fontSettingsPanel" data-font-tab="font-settings" data-i18n="tab.fontSettings">Font settings</button>
                <button type="button" role="tab" aria-selected="false" aria-controls="fontUploadPanel" data-font-tab="font-upload" data-i18n="tab.fontUpload">Font upload</button>
              </div>
              <div class="settings-tab-panel" id="fontSettingsPanel" data-font-panel="font-settings" role="tabpanel">
                <label class="field">
                  <span data-i18n="field.font">Font</span>
                  <select id="fontFamily"></select>
                </label>
                <div class="font-preview" id="fontPreview" aria-label="Font preview" data-i18n-aria="field.fontPreview">
                  <span data-i18n="app.title">Neko Webshell</span>
                  <code>λ ~/app $ ls -la --color=auto 0123456789</code>
                </div>
                <label class="field">
                  <span><span data-i18n="field.fontSize">Font size</span> <output id="fontSizeValue"></output></span>
                  <input id="fontSize" type="range" min="11" max="22" step="1" />
                </label>
                ${renderSettingsDisclosure({ id: "font-rendering", title: "settings.group.fontRendering", description: "settings.group.fontRenderingHelp", content: `
                <label class="field">
                  <span><span data-i18n="field.lineHeight">Line height</span> <output id="lineHeightValue"></output></span>
                  <input id="lineHeight" type="range" min="1.05" max="1.6" step="0.01" />
                </label>
                <div id="fontRenderingSettings"></div>
                ` })}
              </div>
              <div class="settings-tab-panel" id="fontUploadPanel" data-font-panel="font-upload" role="tabpanel" hidden>
                <div class="font-actions">
                  <label class="file-button icon-only-large" aria-label="Upload font" title="Upload font" data-i18n-aria="action.uploadFont" data-i18n-title="action.uploadFont">
                    <input id="fontUpload" type="file" accept=".woff,.woff2,.ttf,.otf,font/woff,font/woff2,font/ttf,font/otf" />
                    <i data-lucide="upload"></i>
                  </label>
                  <button class="command-button icon-only-large" id="removeFont" type="button" aria-label="Remove selected font" title="Remove selected font" data-i18n-aria="action.removeFont" data-i18n-title="action.removeFont">
                    <i data-lucide="trash-2"></i>
                  </button>
                </div>
                <p id="fontStatus" class="field-status"></p>
              </div>
            </section>

  `;
}
