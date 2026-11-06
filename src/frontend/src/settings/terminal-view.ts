import { renderHcHistorySettings } from "../hc/settings-view";
import { renderTerminalDiagnosticsSettings } from "../diagnostics/settings-view";
import { renderHerdrNotificationSettingsView } from "../herdr-notifications/settings-view";
import { renderTerminalControlSettingsView } from "../terminal-control/settings-view";
import { renderSettingsSectionIntro } from "./navigation-view";
import { renderSettingsDisclosure } from "./disclosure-view";

export function renderTerminalSettingsView(): string {
  return `
<section class="settings-section" id="terminalSettingsPanel" data-settings-panel="terminal" role="tabpanel" aria-labelledby="settings-tab-terminal" hidden>
${renderSettingsSectionIntro("terminal")}
              <label class="field">
                <span data-i18n="field.instanceTabDisplay">Instance tabs</span>
                <select id="instanceTabDisplay">
                  <option value="combined" data-i18n="instanceTabDisplay.combined">Show all instances together</option>
                  <option value="by-instance" data-i18n="instanceTabDisplay.byInstance">Show selected instance only</option>
                </select>
              </label>
              <label class="field">
                <span data-i18n="field.cursor">Cursor</span>
                <select id="cursorShape">
                  <option value="block" data-i18n="cursor.block">Block</option>
                  <option value="bar" data-i18n="cursor.bar">Bar</option>
                  <option value="underline" data-i18n="cursor.underline">Underline</option>
                </select>
              </label>
              <label class="switch">
                <input id="cursorBlink" type="checkbox" />
                <span data-i18n="setting.cursorBlink">Cursor blink</span>
              </label>
<div class="settings-disclosures">
${renderSettingsDisclosure({ id: "terminal-sessions", title: "settings.group.sessions", description: "settings.group.sessionsHelp", content: `
              <div class="settings-group" id="sessionBackendSettings" hidden>
                <div class="settings-group-title" data-i18n="section.sessionBackend">Session backend</div>
                <label class="field">
                  <span data-i18n="field.defaultSessionBackend">New tab backend</span>
                  <select id="defaultSessionBackend">
                    <option value="webshell" data-i18n="backend.webshell">WebShell native</option>
                  </select>
                </label>
                ${renderHcHistorySettings()}
                <p id="sessionBackendHelp" class="settings-help" data-i18n="setting.defaultSessionBackendHelp">Applies to new sessions only; existing sessions keep their backend. HC requires an installed hc on a LightOS target. The + button uses this backend. If Herdr already has an engine pane, + creates a new Herdr workspace inside that session.</p>
              </div>
              <label class="switch">
                <input id="autoRestartSessions" type="checkbox" />
                <span data-i18n="setting.autoRestartSessions">Restart sessions after provider restart</span>
              </label>
<div class="settings-group agent-update-settings" id="agentUpdateSettings" hidden></div>
` })}
${renderSettingsDisclosure({ id: "terminal-input", title: "settings.group.input", description: "settings.group.inputHelp", content: `
              <label class="switch">
                <input id="copyOnSelect" type="checkbox" />
                <span data-i18n="setting.copyOnSelect">Copy on select</span>
              </label>
              <label class="switch">
                <input id="useResttyClipboard" type="checkbox" />
                <span data-i18n="setting.useResttyClipboard">Use restty clipboard</span>
              </label>
<div id="terminalControlSettings">${renderTerminalControlSettingsView()}</div>
${renderHerdrNotificationSettingsView()}
` })}
${renderSettingsDisclosure({ id: "terminal-effects", title: "settings.group.effects", description: "settings.group.effectsHelp", content: `
              <div id="terminalShaderSettings" class="terminal-shader-settings"></div>
              <div class="settings-group terminal-background-settings">
                <div class="settings-group-title" data-i18n="section.terminalBackground">Terminal background</div>
                <label class="switch">
                  <input id="terminalBackgroundEnabled" type="checkbox" />
                  <span data-i18n="setting.terminalBackground">Use background image</span>
                </label>
                <div class="background-actions">
                  <label class="file-button" aria-label="Upload terminal background" title="Upload terminal background" data-i18n-aria="action.uploadTerminalBackground" data-i18n-title="action.uploadTerminalBackground">
                    <input id="terminalBackgroundUpload" type="file" accept=".png,.jpg,.jpeg,.webp,image/png,image/jpeg,image/webp" />
                    <i data-lucide="image-plus"></i>
                    <span data-i18n="action.uploadTerminalBackground">Upload terminal background</span>
                  </label>
                  <button class="command-button" id="removeTerminalBackground" type="button" aria-label="Remove terminal background" title="Remove terminal background" data-i18n-aria="action.removeTerminalBackground" data-i18n-title="action.removeTerminalBackground">
                    <i data-lucide="trash-2"></i>
                    <span data-i18n="action.removeTerminalBackground">Remove terminal background</span>
                  </button>
                </div>
                <label class="field">
                  <span><span data-i18n="field.terminalBackgroundOpacity">Background opacity</span> <output id="terminalBackgroundOpacityValue"></output></span>
                  <input id="terminalBackgroundOpacity" type="range" min="0.05" max="0.8" step="0.01" />
                </label>
                <label class="field">
                  <span><span data-i18n="field.terminalBackgroundBlur">Background blur</span> <output id="terminalBackgroundBlurValue"></output></span>
                  <input id="terminalBackgroundBlur" type="range" min="0" max="24" step="1" />
                </label>
                <p id="terminalBackgroundStatus" class="field-status"></p>
              </div>

` })}
${renderSettingsDisclosure({ id: "terminal-history", title: "settings.group.history", description: "settings.group.historyHelp", content: `
              <label class="field">
                <span data-i18n="field.scrollback">Scrollback</span>
                <input id="scrollbackLimit" type="number" min="1000" max="100000" step="1000" />
              </label>
              <label class="field">
                <span data-i18n="field.outputBuffer">Output buffer</span>
                <input id="outputBufferLimit" type="number" min="128" max="20000" step="128" />
              </label>
              <label class="switch">
                <input id="debugMode" type="checkbox" />
                <span data-i18n="setting.debugAdapter">Debug mode</span>
              </label>
${renderTerminalDiagnosticsSettings()}
              <label class="switch">
                <input id="performanceMeterEnabled" type="checkbox" />
                <span data-i18n="setting.performanceMeter">Frame scheduler monitor</span>
              </label>
` })}
</div>
</section>
  `;
}
