import { renderSettingsSectionIntro } from "./navigation-view";
import { renderSettingsDisclosure } from "./disclosure-view";

export function renderThemesSettingsView(): string {
  return `
            <section class="settings-section" id="themeSettingsPanel" data-settings-panel="themes" role="tabpanel" aria-labelledby="settings-tab-themes" hidden>
              ${renderSettingsSectionIntro("themes")}
              <label class="field">
                <span data-i18n="field.theme">Terminal theme</span>
                <select id="themeSelect"></select>
              </label>
              ${renderSettingsDisclosure({ id: "theme-editor", title: "settings.group.themeEditor", description: "settings.group.themeEditorHelp", content: `
              <a class="settings-link" href="https://ghostty-style.vercel.app" target="_blank" rel="noreferrer">
                <i data-lucide="external-link"></i>
                <span data-i18n="theme.gallery">Ghostty Style Gallery</span>
              </a>
              <label class="field">
                <span data-i18n="field.themeName">Theme name</span>
                <input id="customThemeName" type="text" autocomplete="off" spellcheck="false" />
              </label>
              <label class="field">
                <span data-i18n="field.themeSource">Ghostty theme</span>
                <textarea id="customThemeSource" spellcheck="false" rows="7"></textarea>
              </label>
              <div class="theme-actions">
                <button class="command-button" id="saveTheme" type="button">
                  <i data-lucide="save"></i>
                  <span data-i18n="action.saveTheme">Save custom theme</span>
                </button>
                <button class="command-button" id="removeTheme" type="button">
                  <i data-lucide="trash-2"></i>
                  <span data-i18n="action.removeTheme">Remove custom theme</span>
                </button>
              </div>
              <p id="themeStatus" class="field-status"></p>
              ` })}
            </section>

  `;
}
