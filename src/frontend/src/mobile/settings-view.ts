import { renderSettingsDisclosure } from "../settings/disclosure-view";
import { renderMobileClockSettingsView } from "./settings/clock-view";
import { renderMobileQuickPhraseSettingsView } from "./settings/quick-phrase-view";
import { renderMobileTouchSettingsView } from "./settings/touch-view";
import { renderMobileKeyboardLayoutSettingsView } from "./settings/keyboard-layout-view";
import { renderSettingsSectionIntro } from "../settings/navigation-view";

export function renderMobileSettingsView(): string {
  return `
            <section class="settings-section" id="mobileSettingsPanel" data-settings-panel="mobile" role="tabpanel" aria-labelledby="settings-tab-mobile" hidden>
              ${renderSettingsSectionIntro("mobile")}
              ${renderMobileKeyboardLayoutSettingsView()}
              <div class="settings-disclosures">
                ${renderSettingsDisclosure({ id: "mobile-touch", title: "section.mobileTouchKeyboard", description: "settings.group.mobileTouchHelp", content: renderMobileTouchSettingsView() })}
                ${renderSettingsDisclosure({ id: "mobile-phrases", title: "section.mobileQuickInput", description: "settings.group.mobilePhrasesHelp", content: renderMobileQuickPhraseSettingsView() })}
                ${renderSettingsDisclosure({ id: "mobile-clock", title: "section.mobileClock", description: "settings.group.mobileClockHelp", content: renderMobileClockSettingsView() })}
              </div>
            </section>
  `;
}
