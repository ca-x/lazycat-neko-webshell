import { renderFontsSettingsView } from "./settings/fonts-view";
import { renderRemoteControlSettings } from "./remote-control/settings-view";
import { renderThemesSettingsView } from "./settings/themes-view";
import { renderTerminalSettingsView } from "./settings/terminal-view";
import { renderSettingsDisclosure } from "./settings/disclosure-view";
import { renderSettingsNavigation, renderSettingsSectionIntro } from "./settings/navigation-view";
import "./diagnostics/settings.css";
import { qs } from "./utils";
import { renderAboutDialog } from "./about-view";
import { renderHerdrJumpShell } from "./herdr-jump-shell";
import { renderMobileKeyboardView } from "./mobile/keyboard-view";
import { renderMobileWorkspaceOverviewShell } from "./mobile/workspace-overview-view";
import { renderMobileSettingsView } from "./mobile/settings-view";

export type ShellElements = {
  webshell: HTMLElement;
  topbar: HTMLElement;
  instanceList: HTMLDivElement;
  instanceSwitcher: HTMLDivElement;
  instanceButton: HTMLButtonElement;
  instanceMenu: HTMLDivElement;
  instanceStatusDot: HTMLSpanElement;
  refreshInstances: HTMLButtonElement;
  newTabButton: HTMLButtonElement;
  newTabShell: HTMLDivElement;
  newTabMenu: HTMLDivElement;
  emptyNewTab: HTMLButtonElement;
  statusLine: HTMLParagraphElement;
  targetLabel: HTMLElement;
  tabList: HTMLDivElement;
  herdrWorkspaceSwitcher: HTMLDivElement;
  herdrWorkspaceButton: HTMLButtonElement;
  herdrWorkspaceMenu: HTMLDivElement;
  herdrWorkspaceMenuList: HTMLDivElement;
  herdrWorkspaceMenuStatus: HTMLParagraphElement;
  herdrDock: HTMLElement;
  herdrWorkspaceList: HTMLDivElement;
  herdrTabList: HTMLDivElement;
  herdrStatus: HTMLParagraphElement;
  herdrProtocolNotice: HTMLSpanElement;
  herdrRefresh: HTMLButtonElement;
  herdrNewWorkspace: HTMLButtonElement;
  herdrNewTab: HTMLButtonElement;
  herdrMoreButton: HTMLButtonElement;
  herdrMoreMenu: HTMLDivElement;
  herdrCloseWorkspace: HTMLButtonElement;
  herdrRuntimeGuard: HTMLDivElement;
  herdrRuntimeGuardMessage: HTMLParagraphElement;
  herdrHandoff: HTMLButtonElement;
  terminalStage: HTMLDivElement;
  mobileShortcuts: HTMLDivElement;
  mobileWorkspaceOverview: HTMLDivElement;
  terminalInputActionsSurface: HTMLDivElement;
  mobileShortcutClock: HTMLSpanElement;
  emptyState: HTMLDivElement;
  homeButton: HTMLButtonElement;
  settingsButton: HTMLButtonElement;
  settingsMenu: HTMLDivElement;
  openAboutItem: HTMLButtonElement;
  openSettingsItem: HTMLButtonElement;
  openPluginsItem: HTMLButtonElement;
  openShortcutHelpItem: HTMLButtonElement;
  fitTerminalItem: HTMLButtonElement;
  notificationsShell: HTMLDivElement;
  notificationsButton: HTMLButtonElement;
  notificationCount: HTMLSpanElement;
  notificationsMenu: HTMLDivElement;
  notificationList: HTMLDivElement;
  notificationModal: HTMLDivElement;
  notificationModalBody: HTMLDivElement;
  pluginsButton: HTMLButtonElement;
  pluginSidebar: HTMLElement;
  closePluginSidebar: HTMLButtonElement;
  pluginToolTabs: HTMLDivElement;
  pluginToolBody: HTMLDivElement;
  pluginToolStatus: HTMLElement;
  whiteNoiseFloatingControls: HTMLDivElement;
  closeSettings: HTMLButtonElement;
  settingsPage: HTMLElement;
  agentUpdateSettings: HTMLElement;
  agentUpdateButton: HTMLButtonElement;
  settingsTabs: HTMLDivElement;
  fontTabs: HTMLDivElement;
  pluginList: HTMLDivElement;
  voiceSettings: HTMLDivElement;
  pluginStatus: HTMLElement;
  refreshPlugins: HTMLButtonElement;
  localeSelect: HTMLSelectElement;
  interfaceStyleSelect: HTMLSelectElement;
  sessionBackendSettings: HTMLDivElement;
  defaultSessionBackend: HTMLSelectElement;
  hcHistorySettings: HTMLDivElement;
  hcHistoryLines: HTMLInputElement;
  instanceTabDisplay: HTMLSelectElement;
  sessionBackendHelp: HTMLParagraphElement;
  sshProfileSettings: HTMLDivElement;
  herdrHighlightSettings: HTMLDetailsElement;
  herdrActiveBackgroundDark: HTMLInputElement;
  herdrActiveBackgroundLight: HTMLInputElement;
  themeSelect: HTMLSelectElement;
  customThemeName: HTMLInputElement;
  customThemeSource: HTMLTextAreaElement;
  saveTheme: HTMLButtonElement;
  removeTheme: HTMLButtonElement;
  themeStatus: HTMLElement;
  fontFamily: HTMLSelectElement;
  fontPreview: HTMLDivElement;
  fontRenderingSettings: HTMLDivElement;
  tabLayout: HTMLSelectElement;
  fontUpload: HTMLInputElement;
  removeFont: HTMLButtonElement;
  fontStatus: HTMLElement;
  fontSize: HTMLInputElement;
  fontSizeValue: HTMLOutputElement;
  lineHeight: HTMLInputElement;
  lineHeightValue: HTMLOutputElement;
  scrollbackLimit: HTMLInputElement;
  outputBufferLimit: HTMLInputElement;
  terminalControlSettings: HTMLDivElement;
  terminalSingleControllerMode: HTMLInputElement;
  terminalBlurObservers: HTMLInputElement;
  herdrNotificationSettings: HTMLDivElement;
  herdrLazycatNotifications: HTMLInputElement;
  terminalBackgroundEnabled: HTMLInputElement;
  terminalBackgroundUpload: HTMLInputElement;
  removeTerminalBackground: HTMLButtonElement;
  terminalBackgroundOpacity: HTMLInputElement;
  terminalBackgroundOpacityValue: HTMLOutputElement;
  terminalBackgroundBlur: HTMLInputElement;
  terminalBackgroundBlurValue: HTMLOutputElement;
  terminalBackgroundStatus: HTMLElement;
  terminalShaderSettings: HTMLDivElement;
  cursorBlink: HTMLInputElement;
  cursorShape: HTMLSelectElement;
  copyOnSelect: HTMLInputElement;
  useResttyClipboard: HTMLInputElement;
  touchSelectionMode: HTMLSelectElement;
  preventMobileKeyboardAutoOpen: HTMLInputElement;
  mobileClockEnabled: HTMLInputElement;
  mobileClockUse24Hour: HTMLInputElement;
  mobileClockShowPeriod: HTMLInputElement;
  mobileQuickPhraseSettings: HTMLDivElement;
  mobileKeyboardLayoutSettings: HTMLDivElement;
  mobileQuickPhraseList: HTMLDivElement;
  mobileQuickPhraseLabel: HTMLInputElement;
  mobileQuickPhraseGroup: HTMLInputElement;
  mobileQuickPhraseText: HTMLTextAreaElement;
  mobileQuickPhraseSendEnter: HTMLInputElement;
  mobileQuickPhraseSave: HTMLButtonElement;
  mobileQuickPhraseCancel: HTMLButtonElement;
  mobileQuickPhraseStatus: HTMLElement;
  autoRestartSessions: HTMLInputElement;
  debugMode: HTMLInputElement;
  performanceMeterEnabled: HTMLInputElement;
  shortcutHelpButton: HTMLButtonElement;
  shortcutHelp: HTMLDivElement;
  shortcutHelpClose: HTMLButtonElement;
  aboutDialog: HTMLDivElement;
  aboutClose: HTMLButtonElement;
  terminalControlOverlay: HTMLDivElement;
  fitTerminal: HTMLButtonElement;
};

export function renderShell(app: HTMLElement): ShellElements {
  app.innerHTML = `
    <main class="webshell" id="webshell" aria-label="Neko Webshell workspace" data-i18n-aria="app.title">
      <header class="topbar" aria-label="Terminal controls" data-i18n-aria="app.title">
        <div class="tabs-shell">
          <div id="tabList" class="tab-list" role="tablist" aria-label="Terminal tabs" data-i18n-aria="action.newTab"></div>
          <div class="new-tab-shell" id="newTabShell">
            <button class="tab-add" id="newTabButton" type="button" aria-haspopup="menu" aria-expanded="false" aria-label="New terminal tab" title="New terminal tab" data-i18n-aria="action.newTab" data-i18n-title="action.newTab">
              <i data-lucide="plus"></i>
            </button>
            <div class="new-tab-menu" id="newTabMenu" role="menu" aria-label="New terminal tab" data-i18n-aria="action.newTab" hidden></div>
          </div>
        </div>
        <div class="topbar-actions">
          <div class="instance-switcher" id="instanceSwitcher">
            <button class="icon-button status-icon" id="instanceButton" type="button" aria-haspopup="listbox" aria-expanded="false" aria-label="Switch instance" title="Switch instance" data-i18n-aria="action.switchInstance" data-i18n-title="action.switchInstance">
              <span class="status-dot" id="instanceStatusDot" data-status="unknown"></span>
              <i data-lucide="server"></i>
              <span id="targetLabel" class="sr-only" data-i18n="status.noTarget">No instance selected</span>
            </button>
            <div class="switcher-menu" id="instanceMenu" hidden>
              <div class="menu-head">
                <span data-i18n="menu.instances">Instances</span>
                <button class="icon-button" id="refreshInstances" type="button" aria-label="Refresh instances" title="Refresh instances" data-i18n-aria="action.refreshInstances" data-i18n-title="action.refreshInstances">
                  <i data-lucide="refresh-cw"></i>
                </button>
              </div>
              <div id="instanceList" class="instance-list" role="listbox" aria-label="Running instances" aria-live="polite"></div>
            </div>
          </div>
          <button class="icon-button" id="fitTerminal" type="button" aria-label="Full screen" title="Full screen" data-i18n-aria="action.fullscreen" data-i18n-title="action.fullscreen">
            <i data-lucide="maximize"></i>
          </button>
          <div class="shortcut-help-shell" id="shortcutHelpShell">
            <button class="icon-button" id="shortcutHelpButton" type="button" aria-haspopup="dialog" aria-expanded="false" aria-label="Keyboard shortcuts" title="Keyboard shortcuts" data-i18n-aria="action.shortcutHelp" data-i18n-title="action.shortcutHelp">
              <i data-lucide="keyboard"></i>
            </button>
            <div class="shortcut-help" id="shortcutHelp" hidden>
              <div class="shortcut-help-dialog" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" data-i18n-aria="action.shortcutHelp">
                <div class="shortcut-help-head">
                  <strong data-i18n="section.shortcuts">Shortcuts</strong>
                  <button class="icon-button" id="shortcutHelpClose" type="button" aria-label="Close settings" title="Close settings" data-i18n-aria="action.closeSettings" data-i18n-title="action.closeSettings">
                    <i data-lucide="x"></i>
                  </button>
                </div>
                <div class="shortcut-help-grid">
                  <section>
                    <h3 data-i18n="section.desktopShortcuts">Desktop</h3>
                    <dl>
                      <div><dt><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>T</kbd></dt><dd data-i18n="shortcut.newTab">New terminal tab</dd></div>
                      <div><dt><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>W</kbd></dt><dd data-i18n="shortcut.closeTab">Close tab</dd></div>
                      <div><dt><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>↑/↓/←/→</kbd></dt><dd data-i18n="shortcut.splitPane">Split pane</dd></div>
                      <div><dt><kbd>Ctrl</kbd><kbd>+</kbd> / <kbd>Ctrl</kbd><kbd>-</kbd></dt><dd data-i18n="shortcut.zoomFont">Adjust terminal font</dd></div>
                      <div><dt><kbd>Ctrl</kbd><kbd>0</kbd></dt><dd data-i18n="shortcut.resetFont">Reset terminal font</dd></div>
                      <div><dt><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>C/V</kbd></dt><dd data-i18n="shortcut.copyPaste">Copy or paste</dd></div>
                      <div><dt><kbd>Ctrl/⌘</kbd><kbd>Alt/⌥</kbd><kbd>V</kbd></dt><dd data-i18n="secret.shortcutHelp">Secret file (default; change in its panel)</dd></div>
                    </dl>
                  </section>
                  <section>
                    <h3 data-i18n="section.mobileShortcuts">Mobile</h3>
                    <dl>
                      <div><dt><kbd>Ops</kbd><kbd>A+</kbd>/<kbd>A-</kbd></dt><dd data-i18n="shortcut.mobileFont">Adjust terminal font</dd></div>
                      <div><dt><kbd>Ops</kbd><kbd>Tab</kbd></dt><dd data-i18n="shortcut.mobileTab">Switch or create tabs</dd></div>
                      <div><dt><kbd>Main</kbd><kbd>Ctrl</kbd><kbd>Tab</kbd></dt><dd data-i18n="shortcut.mobileKeys">Send terminal keys</dd></div>
                      <div><dt><kbd>double tap</kbd></dt><dd data-i18n="shortcut.mobileKeyboard">Open system keyboard</dd></div>
                    </dl>
                  </section>
                </div>
              </div>
            </div>
          </div>
          <div class="notifications-shell" id="notificationsShell" hidden>
            <button class="icon-button notification-button" id="notificationsButton" type="button" aria-haspopup="dialog" aria-expanded="false" aria-label="Notifications" title="Notifications" data-i18n-aria="section.notifications" data-i18n-title="section.notifications">
              <i data-lucide="bell"></i>
              <span class="notification-count" id="notificationCount" hidden></span>
            </button>
            <div class="notifications-menu" id="notificationsMenu" role="dialog" aria-label="Notifications" data-i18n-aria="section.notifications" hidden>
              <div class="menu-head">
                <span data-i18n="section.notifications">Notifications</span>
              </div>
              <div class="notification-list" id="notificationList" role="list" aria-live="polite"></div>
            </div>
          </div>
          <button class="agent-update-entry" id="agentUpdateButton" type="button" hidden aria-label="Update available" data-i18n-aria="agentUpdate.available"><i data-lucide="download" aria-hidden="true"></i><span data-i18n="agentUpdate.available">Update available</span></button>
          <button class="icon-button" id="pluginsButton" type="button" aria-haspopup="dialog" aria-expanded="false" aria-label="Tools" title="Tools" data-i18n-aria="section.plugins" data-i18n-title="section.plugins">
            <i data-lucide="plug"></i>
          </button>
          <div class="settings-menu-shell" id="settingsMenuShell">
            <button class="icon-button" id="settingsButton" type="button" aria-haspopup="menu" aria-expanded="false" aria-label="Settings menu" title="Settings menu" data-i18n-aria="action.settingsMenu" data-i18n-title="action.settingsMenu">
              <i data-lucide="menu"></i>
            </button>
            <div class="settings-menu" id="settingsMenu" role="menu" aria-label="Settings menu" data-i18n-aria="action.settingsMenu" hidden>
              <button id="openPluginsItem" type="button" role="menuitem">
                <i data-lucide="plug"></i>
                <span data-i18n="section.plugins">Tools</span>
              </button>
              <button id="openShortcutHelpItem" type="button" role="menuitem">
                <i data-lucide="keyboard"></i>
                <span data-i18n="action.shortcutHelp">Keyboard shortcuts</span>
              </button>
              <button id="openAboutItem" type="button" role="menuitem">
                <i data-lucide="badge-info"></i>
                <span data-i18n="action.about">About</span>
              </button>
              <button id="fitTerminalItem" type="button" role="menuitem">
                <i data-lucide="maximize"></i>
                <span data-i18n="action.fullscreen">Full screen</span>
              </button>
              <button id="openSettingsItem" type="button" role="menuitem">
                <i data-lucide="settings"></i>
                <span data-i18n="action.settings">Settings</span>
              </button>
              <button id="homeButton" type="button" role="menuitem">
                <i data-lucide="house"></i>
                <span data-i18n="action.lightosHome">LightOS home</span>
              </button>
            </div>
          </div>
        </div>
      </header>

      ${renderAboutDialog()}

      ${renderHerdrJumpShell()}

      <section id="terminalStage" class="terminal-stage" aria-label="Terminal" data-i18n-aria="app.title">
        <div class="empty-state" id="emptyState">
          <button class="command-button primary icon-only-large" id="emptyNewTab" type="button" aria-label="New terminal tab" title="New terminal tab" data-i18n-aria="action.newTab" data-i18n-title="action.newTab">
            <i data-lucide="square-plus"></i>
          </button>
          <p id="statusLine" aria-live="polite" data-i18n="status.idle">Idle</p>
        </div>
        <div id="terminalControlOverlay" class="terminal-control-overlay" hidden></div>
      </section>

      <div class="notification-modal" id="notificationModal" hidden>
        <div class="notification-modal-dialog" role="dialog" aria-modal="true" aria-label="Notification" data-i18n-aria="section.notifications">
          <div id="notificationModalBody"></div>
        </div>
      </div>

      ${renderMobileKeyboardView()}
      ${renderMobileWorkspaceOverviewShell()}

      <div class="terminal-input-actions-surface" id="terminalInputActionsSurface" hidden></div>

      <aside class="plugin-sidebar" id="pluginSidebar" aria-label="Tools" data-i18n-aria="section.plugins" hidden>
        <header class="plugin-sidebar-header">
          <strong data-i18n="section.plugins">Tools</strong>
          <button class="icon-button" id="closePluginSidebar" type="button" aria-label="Close tools" title="Close tools" data-i18n-aria="action.closePlugins" data-i18n-title="action.closePlugins">
            <i data-lucide="x"></i>
          </button>
        </header>
        <div class="plugin-tool-tabs" id="pluginToolTabs" role="tablist" aria-label="Tools" data-i18n-aria="section.plugins"></div>
        <div class="plugin-tool-body" id="pluginToolBody"></div>
        <p id="pluginToolStatus" class="field-status"></p>
      </aside>

      <div id="whiteNoiseFloatingControls" hidden></div>

      <section class="settings-page" id="settingsPage" hidden aria-label="Settings" data-i18n-aria="action.settings">
        <div class="settings-dialog" role="dialog" aria-modal="true" aria-labelledby="settingsTitle">
          <header class="settings-header">
            <div>
              <h2 id="settingsTitle" data-i18n="action.settings">Settings</h2>
              <p data-i18n="settings.intro"></p>
            </div>
            <button class="icon-button" id="closeSettings" type="button" aria-label="Close settings" title="Close settings" data-i18n-aria="action.closeSettings" data-i18n-title="action.closeSettings">
              <i data-lucide="x"></i>
            </button>
          </header>

          <div class="settings-layout">
            ${renderSettingsNavigation()}

          <div class="settings-panels">
            <section class="settings-section" id="appearanceSettingsPanel" data-settings-panel="appearance" role="tabpanel" aria-labelledby="settings-tab-appearance">
              ${renderSettingsSectionIntro("appearance")}
              <label class="field">
                <span data-i18n="field.language">Language</span>
                <select id="localeSelect">
                  <option value="auto" data-i18n="locale.auto">Auto</option>
                  <option value="en" data-i18n="locale.en">English</option>
                  <option value="zh-CN" data-i18n="locale.zhCN">Chinese</option>
                </select>
              </label>
              <label class="field">
                <span data-i18n="field.interfaceStyle">Interface style</span>
                <select id="interfaceStyleSelect">
                  <option value="steel" data-i18n="interfaceStyle.steel">Steel</option>
                  <option value="glass" data-i18n="interfaceStyle.glass">Glass</option>
                  <option value="brass" data-i18n="interfaceStyle.brass">Brass</option>
                  <option value="spectrum" data-i18n="interfaceStyle.spectrum">Spectrum</option>
                  <option value="geek" data-i18n="interfaceStyle.geek">Geek</option>
                  <option value="porcelain" data-i18n="interfaceStyle.porcelain">Porcelain</option>
                  <option value="frost" data-i18n="interfaceStyle.frost">Frost</option>
                  <option value="champagne" data-i18n="interfaceStyle.champagne">Champagne</option>
                  <option value="candy" data-i18n="interfaceStyle.candy">Candy</option>
                  <option value="lab" data-i18n="interfaceStyle.lab">Lab</option>
                </select>
              </label>
              <label class="field">
                <span data-i18n="field.tabs">Tabs</span>
                <select id="tabLayout">
                  <option value="horizontal" data-i18n="layout.horizontal">Horizontal</option>
                  <option value="vertical" data-i18n="layout.vertical">Vertical</option>
                </select>
              </label>
              ${renderSettingsDisclosure({ id: "herdrHighlightSettings", title: "section.herdrHighlight", description: "settings.group.herdrHighlightHelp", hidden: true, content: `
                <label class="field color-field">
                  <span data-i18n="field.herdrActiveBackgroundDark">Dark highlight</span>
                  <input id="herdrActiveBackgroundDark" type="color" />
                </label>
                <label class="field color-field">
                  <span data-i18n="field.herdrActiveBackgroundLight">Light highlight</span>
                  <input id="herdrActiveBackgroundLight" type="color" />
                </label>
              ` })}

            </section>

            ${renderTerminalSettingsView()}

            <section class="settings-section" id="remoteHostsSettingsPanel" data-settings-panel="remote-hosts" role="tabpanel" aria-labelledby="settings-tab-remote-hosts" hidden>
              ${renderSettingsSectionIntro("remote-hosts")}
              <div class="settings-group ssh-profile-settings" id="sshProfileSettings"></div>
            </section>

            ${renderRemoteControlSettings()}

            ${renderMobileSettingsView()}

            ${renderFontsSettingsView()}

            ${renderThemesSettingsView()}

            <section class="settings-section" id="voiceSettingsPanel" data-settings-panel="voice" role="tabpanel" aria-labelledby="settings-tab-voice" hidden>
              ${renderSettingsSectionIntro("voice")}
              <div id="voiceSettings"></div>
            </section>

            <section class="settings-section" id="pluginSettingsPanel" data-settings-panel="plugins" role="tabpanel" aria-labelledby="settings-tab-plugins" hidden>
              ${renderSettingsSectionIntro("plugins")}
              <div class="settings-section-head">
                <div class="settings-group-title" data-i18n="section.plugins">Tools</div>
                <button class="icon-button" id="refreshPlugins" type="button" aria-label="Refresh tools" title="Refresh tools" data-i18n-aria="action.refreshPlugins" data-i18n-title="action.refreshPlugins">
                  <i data-lucide="refresh-cw"></i>
                </button>
              </div>
              <div class="plugin-list" id="pluginList" role="list"></div>
              <p id="pluginStatus" class="field-status" role="status" aria-live="polite"></p>
            </section>
          </div>
          </div>
        </div>
      </section>

    </main>
  `;

  return {
    webshell: qs<HTMLElement>("#webshell"),
    topbar: qs<HTMLElement>(".topbar"),
    instanceList: qs<HTMLDivElement>("#instanceList"),
    instanceSwitcher: qs<HTMLDivElement>("#instanceSwitcher"),
    instanceButton: qs<HTMLButtonElement>("#instanceButton"),
    instanceMenu: qs<HTMLDivElement>("#instanceMenu"),
    instanceStatusDot: qs<HTMLSpanElement>("#instanceStatusDot"),
    refreshInstances: qs<HTMLButtonElement>("#refreshInstances"),
    newTabButton: qs<HTMLButtonElement>("#newTabButton"),
    newTabShell: qs<HTMLDivElement>("#newTabShell"),
    newTabMenu: qs<HTMLDivElement>("#newTabMenu"),
    emptyNewTab: qs<HTMLButtonElement>("#emptyNewTab"),
    statusLine: qs<HTMLParagraphElement>("#statusLine"),
    targetLabel: qs<HTMLElement>("#targetLabel"),
    tabList: qs<HTMLDivElement>("#tabList"),
    herdrWorkspaceSwitcher: qs<HTMLDivElement>("#herdrWorkspaceSwitcher"),
    herdrWorkspaceButton: qs<HTMLButtonElement>("#herdrWorkspaceButton"),
    herdrWorkspaceMenu: qs<HTMLDivElement>("#herdrWorkspaceMenu"),
    herdrWorkspaceMenuList: qs<HTMLDivElement>("#herdrWorkspaceMenuList"),
    herdrWorkspaceMenuStatus: qs<HTMLParagraphElement>("#herdrWorkspaceMenuStatus"),
    herdrDock: qs<HTMLElement>("#herdrDock"),
    herdrWorkspaceList: qs<HTMLDivElement>("#herdrWorkspaceList"),
    herdrTabList: qs<HTMLDivElement>("#herdrTabList"),
    herdrStatus: qs<HTMLParagraphElement>("#herdrStatus"),
    herdrProtocolNotice: qs<HTMLSpanElement>("#herdrProtocolNotice"),
    herdrRefresh: qs<HTMLButtonElement>("#herdrRefresh"),
    herdrNewWorkspace: qs<HTMLButtonElement>("#herdrNewWorkspace"),
    herdrNewTab: qs<HTMLButtonElement>("#herdrNewTab"),
    herdrMoreButton: qs<HTMLButtonElement>("#herdrMoreButton"),
    herdrMoreMenu: qs<HTMLDivElement>("#herdrMoreMenu"),
    herdrCloseWorkspace: qs<HTMLButtonElement>("#herdrCloseWorkspace"),
    herdrRuntimeGuard: qs<HTMLDivElement>("#herdrRuntimeGuard"),
    herdrRuntimeGuardMessage: qs<HTMLParagraphElement>("#herdrRuntimeGuardMessage"),
    herdrHandoff: qs<HTMLButtonElement>("#herdrHandoff"),
    terminalStage: qs<HTMLDivElement>("#terminalStage"),
    mobileShortcuts: qs<HTMLDivElement>("#mobileShortcuts"),
    mobileWorkspaceOverview: qs<HTMLDivElement>("#mobileWorkspaceOverview"),
    terminalInputActionsSurface: qs<HTMLDivElement>("#terminalInputActionsSurface"),
    mobileShortcutClock: qs<HTMLSpanElement>("#mobileShortcutClock"),
    emptyState: qs<HTMLDivElement>("#emptyState"),
    homeButton: qs<HTMLButtonElement>("#homeButton"),
    settingsButton: qs<HTMLButtonElement>("#settingsButton"),
    settingsMenu: qs<HTMLDivElement>("#settingsMenu"),
    openAboutItem: qs<HTMLButtonElement>("#openAboutItem"),
    openSettingsItem: qs<HTMLButtonElement>("#openSettingsItem"),
    openPluginsItem: qs<HTMLButtonElement>("#openPluginsItem"),
    openShortcutHelpItem: qs<HTMLButtonElement>("#openShortcutHelpItem"),
    fitTerminalItem: qs<HTMLButtonElement>("#fitTerminalItem"),
    notificationsShell: qs<HTMLDivElement>("#notificationsShell"),
    notificationsButton: qs<HTMLButtonElement>("#notificationsButton"),
    notificationCount: qs<HTMLSpanElement>("#notificationCount"),
    notificationsMenu: qs<HTMLDivElement>("#notificationsMenu"),
    notificationList: qs<HTMLDivElement>("#notificationList"),
    notificationModal: qs<HTMLDivElement>("#notificationModal"),
    notificationModalBody: qs<HTMLDivElement>("#notificationModalBody"),
    pluginsButton: qs<HTMLButtonElement>("#pluginsButton"),
    pluginSidebar: qs<HTMLElement>("#pluginSidebar"),
    closePluginSidebar: qs<HTMLButtonElement>("#closePluginSidebar"),
    pluginToolTabs: qs<HTMLDivElement>("#pluginToolTabs"),
    pluginToolBody: qs<HTMLDivElement>("#pluginToolBody"),
    pluginToolStatus: qs<HTMLElement>("#pluginToolStatus"),
    whiteNoiseFloatingControls: qs<HTMLDivElement>("#whiteNoiseFloatingControls"),
    closeSettings: qs<HTMLButtonElement>("#closeSettings"),
    settingsPage: qs<HTMLElement>("#settingsPage"),
    agentUpdateSettings: qs<HTMLElement>("#agentUpdateSettings"),
    agentUpdateButton: qs<HTMLButtonElement>("#agentUpdateButton"),
    settingsTabs: qs<HTMLDivElement>("#settingsTabs"),
    fontTabs: qs<HTMLDivElement>("#fontTabs"),
    pluginList: qs<HTMLDivElement>("#pluginList"),
    voiceSettings: qs<HTMLDivElement>("#voiceSettings"),
    pluginStatus: qs<HTMLElement>("#pluginStatus"),
    refreshPlugins: qs<HTMLButtonElement>("#refreshPlugins"),
    localeSelect: qs<HTMLSelectElement>("#localeSelect"),
    interfaceStyleSelect: qs<HTMLSelectElement>("#interfaceStyleSelect"),
    sessionBackendSettings: qs<HTMLDivElement>("#sessionBackendSettings"),
    defaultSessionBackend: qs<HTMLSelectElement>("#defaultSessionBackend"),
    hcHistorySettings: qs<HTMLDivElement>("#hcHistorySettings"),
    hcHistoryLines: qs<HTMLInputElement>("#hcHistoryLines"),
    instanceTabDisplay: qs<HTMLSelectElement>("#instanceTabDisplay"),
    sessionBackendHelp: qs<HTMLParagraphElement>("#sessionBackendHelp"),
    sshProfileSettings: qs<HTMLDivElement>("#sshProfileSettings"),
    herdrHighlightSettings: qs<HTMLDetailsElement>("#herdrHighlightSettings"),
    herdrActiveBackgroundDark: qs<HTMLInputElement>("#herdrActiveBackgroundDark"),
    herdrActiveBackgroundLight: qs<HTMLInputElement>("#herdrActiveBackgroundLight"),
    themeSelect: qs<HTMLSelectElement>("#themeSelect"),
    customThemeName: qs<HTMLInputElement>("#customThemeName"),
    customThemeSource: qs<HTMLTextAreaElement>("#customThemeSource"),
    saveTheme: qs<HTMLButtonElement>("#saveTheme"),
    removeTheme: qs<HTMLButtonElement>("#removeTheme"),
    themeStatus: qs<HTMLElement>("#themeStatus"),
    fontFamily: qs<HTMLSelectElement>("#fontFamily"),
    fontPreview: qs<HTMLDivElement>("#fontPreview"),
    fontRenderingSettings: qs<HTMLDivElement>("#fontRenderingSettings"),
    tabLayout: qs<HTMLSelectElement>("#tabLayout"),
    fontUpload: qs<HTMLInputElement>("#fontUpload"),
    removeFont: qs<HTMLButtonElement>("#removeFont"),
    fontStatus: qs<HTMLElement>("#fontStatus"),
    fontSize: qs<HTMLInputElement>("#fontSize"),
    fontSizeValue: qs<HTMLOutputElement>("#fontSizeValue"),
    lineHeight: qs<HTMLInputElement>("#lineHeight"),
    lineHeightValue: qs<HTMLOutputElement>("#lineHeightValue"),
    scrollbackLimit: qs<HTMLInputElement>("#scrollbackLimit"),
    outputBufferLimit: qs<HTMLInputElement>("#outputBufferLimit"),
    terminalControlSettings: qs<HTMLDivElement>("#terminalControlSettings"),
    terminalSingleControllerMode: qs<HTMLInputElement>("#terminalSingleControllerMode"),
    terminalBlurObservers: qs<HTMLInputElement>("#terminalBlurObservers"),
    herdrNotificationSettings: qs<HTMLDivElement>("#herdrNotificationSettings"),
    herdrLazycatNotifications: qs<HTMLInputElement>("#herdrLazycatNotifications"),
    terminalBackgroundEnabled: qs<HTMLInputElement>("#terminalBackgroundEnabled"),
    terminalBackgroundUpload: qs<HTMLInputElement>("#terminalBackgroundUpload"),
    removeTerminalBackground: qs<HTMLButtonElement>("#removeTerminalBackground"),
    terminalBackgroundOpacity: qs<HTMLInputElement>("#terminalBackgroundOpacity"),
    terminalBackgroundOpacityValue: qs<HTMLOutputElement>("#terminalBackgroundOpacityValue"),
    terminalBackgroundBlur: qs<HTMLInputElement>("#terminalBackgroundBlur"),
    terminalBackgroundBlurValue: qs<HTMLOutputElement>("#terminalBackgroundBlurValue"),
    terminalBackgroundStatus: qs<HTMLElement>("#terminalBackgroundStatus"),
    terminalShaderSettings: qs<HTMLDivElement>("#terminalShaderSettings"),
    cursorBlink: qs<HTMLInputElement>("#cursorBlink"),
    cursorShape: qs<HTMLSelectElement>("#cursorShape"),
    copyOnSelect: qs<HTMLInputElement>("#copyOnSelect"),
    useResttyClipboard: qs<HTMLInputElement>("#useResttyClipboard"),
    touchSelectionMode: qs<HTMLSelectElement>("#touchSelectionMode"),
    preventMobileKeyboardAutoOpen: qs<HTMLInputElement>("#preventMobileKeyboardAutoOpen"),
    mobileClockEnabled: qs<HTMLInputElement>("#mobileClockEnabled"),
    mobileClockUse24Hour: qs<HTMLInputElement>("#mobileClockUse24Hour"),
    mobileClockShowPeriod: qs<HTMLInputElement>("#mobileClockShowPeriod"),
    mobileQuickPhraseSettings: qs<HTMLDivElement>("#mobileQuickPhraseSettings"),
    mobileKeyboardLayoutSettings: qs<HTMLDivElement>("#mobileKeyboardLayoutSettings"),
    mobileQuickPhraseList: qs<HTMLDivElement>("#mobileQuickPhraseList"),
    mobileQuickPhraseLabel: qs<HTMLInputElement>("#mobileQuickPhraseLabel"),
    mobileQuickPhraseGroup: qs<HTMLInputElement>("#mobileQuickPhraseGroup"),
    mobileQuickPhraseText: qs<HTMLTextAreaElement>("#mobileQuickPhraseText"),
    mobileQuickPhraseSendEnter: qs<HTMLInputElement>("#mobileQuickPhraseSendEnter"),
    mobileQuickPhraseSave: qs<HTMLButtonElement>("#mobileQuickPhraseSave"),
    mobileQuickPhraseCancel: qs<HTMLButtonElement>("#mobileQuickPhraseCancel"),
    mobileQuickPhraseStatus: qs<HTMLElement>("#mobileQuickPhraseStatus"),
    autoRestartSessions: qs<HTMLInputElement>("#autoRestartSessions"),
    debugMode: qs<HTMLInputElement>("#debugMode"),
    performanceMeterEnabled: qs<HTMLInputElement>("#performanceMeterEnabled"),
    shortcutHelpButton: qs<HTMLButtonElement>("#shortcutHelpButton"),
    shortcutHelp: qs<HTMLDivElement>("#shortcutHelp"),
    shortcutHelpClose: qs<HTMLButtonElement>("#shortcutHelpClose"),
    aboutDialog: qs<HTMLDivElement>("#aboutDialog"),
    aboutClose: qs<HTMLButtonElement>("#aboutClose"),
    terminalControlOverlay: qs<HTMLDivElement>("#terminalControlOverlay"),
    fitTerminal: qs<HTMLButtonElement>("#fitTerminal"),
  };
}
