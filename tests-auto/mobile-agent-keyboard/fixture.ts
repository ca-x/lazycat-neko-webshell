import '../../src/frontend/src/styles.css';
import '../../src/frontend/src/mobile/styles.css';
import '../../src/frontend/src/mobile/keyboard-dock.css';
import '../../src/frontend/src/mobile/settings.css';
import { createIcons, icons } from 'lucide';
import { DEFAULT_SETTINGS } from '../../src/frontend/src/config.ts';
import { translate, type MessageKey } from '../../src/frontend/src/i18n.ts';
import { renderShell } from '../../src/frontend/src/shell.ts';
import type { HerdrBridgeState, TerminalPane } from '../../src/frontend/src/types.ts';
import { createMobileAgentKeyboardContext } from '../../src/frontend/src/mobile/agent-keyboard-context.ts';
import { createMobileKeyboardController } from '../../src/frontend/src/mobile/keyboard-controller.ts';
import { createMobileKeyboardLayoutSettingsController } from '../../src/frontend/src/mobile/keyboard-layout-settings-controller.ts';
import { mobileKeyboardPresetLayout, resolveMobileKeyboardLayout } from '../../src/frontend/src/mobile/keyboard-layout.ts';
import type { MobileKeyboardPresetId } from '../../src/frontend/src/mobile/keyboard-layout-types.ts';
import { createMobileSystemKeyboardController } from '../../src/frontend/src/mobile/system-keyboard-controller.ts';

for (const side of ['top', 'right', 'bottom', 'left']) {
  document.documentElement.style.setProperty(`--lzc-safe-area-inset-${side}`, '0px');
}
const elements = renderShell(document.querySelector<HTMLElement>('#app')!);
elements.emptyState.hidden = true;
elements.targetLabel.textContent = 'Agent keyboard verification · dev';
elements.mobileShortcutClock.textContent = '17:42';
elements.mobileShortcutClock.dataset.compact = 'true';
const settings = structuredClone(DEFAULT_SETTINGS);
let locale: 'zh-CN' | 'en' = 'zh-CN';
let saves = 0;
let contextChanges = 0;
const tr = (key: MessageKey, values?: Record<string, string | number>) => translate(locale, key, values);
const updateIcons = () => createIcons({ icons });
const panes = new Map<string, TerminalPane>();
const inputs = new Map<string, HTMLTextAreaElement>();
function makePane(id: string, backend: 'webshell' | 'herdr' = 'webshell'): TerminalPane {
  const pane = {
    id, sessionId: `session-${id}`, selector: 'dev@owner', title: id, label: id,
    sessionBackend: backend, connectionState: 'connected', replaying: false, closing: false, exited: false,
  } as TerminalPane;
  panes.set(id, pane);
  const input = document.createElement('textarea');
  input.id = `terminal-${id}`;
  input.className = 'agent-keyboard-fixture-input';
  input.setAttribute('aria-label', `Terminal ${id}`);
  input.hidden = true;
  elements.terminalStage.append(input);
  inputs.set(id, input);
  return pane;
}
const nativeA = makePane('native-a');
makePane('native-b');
makePane('herdr', 'herdr');
nativeA.programStatus = [{ id: 'process-1', app: 'codex', state: 'working' }];
let activePane = nativeA;
inputs.get(activePane.id)!.hidden = false;
let herdr = {
  selector: 'dev@owner', available: true, resources_complete: true, focused_pane_id: 'remote-a',
  panes: [
    { pane_id: 'remote-a', agent: 'codex', focused: true, agent_status: 'working' },
    { pane_id: 'remote-b', agent: 'claude', focused: false, agent_status: 'working' },
    { pane_id: 'remote-shell', focused: false, agent_status: '' },
  ],
  agents: [],
} as unknown as HerdrBridgeState;
const output: Array<{ paneId: string; remotePaneId?: string; backend: string; data: string }> = [];
const actions: string[] = [];
const keyboard = createMobileKeyboardController({
  root: elements.mobileShortcuts,
  agentContext: () => context.current(),
  preserveSystemKeyboardState: () => system.preserveState(),
  onKeyInput: (data) => output.push({
    paneId: activePane.id, remotePaneId: activePane.sessionBackend === 'herdr' ? herdr.focused_pane_id : undefined,
    backend: activePane.sessionBackend ?? 'webshell', data,
  }),
  onPasteShortcut: async () => {},
  onAction: async (action) => {
    actions.push(action);
    if (action === 'toggle-system-keyboard') system.toggle();
    if (action === 'toggle-expanded-keyboard') {
      if (!keyboard.isExpanded()) system.dismissForOverlay();
      keyboard.toggleExpanded();
    }
  },
  onPhrase: async () => {}, onLayoutChange: () => {}, tr,
});
const system = createMobileSystemKeyboardController({
  activePane: () => activePane,
  dismissPane: (pane) => inputs.get(pane.id)?.blur(),
  enableAllPanes: () => {},
  focusHardwarePane: (pane) => inputs.get(pane.id)?.blur(),
  focusPane: (pane) => { inputs.get(pane.id)?.focus({ preventScroll: true }); return document.activeElement === inputs.get(pane.id); },
  isPaneKeyboardFocused: (pane) => document.activeElement === inputs.get(pane.id),
  updateToggle: keyboard.updateSystemKeyboardState,
});
const context = createMobileAgentKeyboardContext({
  enabled: () => settings.mobileAgentKeyboardEnabled,
  pane: () => activePane,
  herdr: () => herdr,
  changed: () => { contextChanges++; renderKeyboard(); },
});
function renderKeyboard() {
  keyboard.renderQuickInput({
    phrases: [], tr, preset: settings.mobileKeyboardPreset,
    layout: resolveMobileKeyboardLayout(settings.mobileKeyboardPreset, settings.mobileKeyboardLayout),
  });
  updateIcons();
}
const settingsDialog = document.createElement('dialog');
settingsDialog.id = 'agentKeyboardFixtureSettings';
settingsDialog.append(elements.mobileKeyboardLayoutSettings);
const closeSettings = document.createElement('button');
closeSettings.type = 'button';
closeSettings.id = 'agentKeyboardFixtureCloseSettings';
closeSettings.textContent = 'Close settings';
closeSettings.addEventListener('click', () => settingsDialog.close());
settingsDialog.prepend(closeSettings);
document.body.append(settingsDialog);
const settingsController = createMobileKeyboardLayoutSettingsController({
  root: elements.mobileKeyboardLayoutSettings,
  preset: () => settings.mobileKeyboardPreset,
  layout: () => settings.mobileKeyboardLayout,
  setPreset: (value) => { settings.mobileKeyboardPreset = value; },
  setLayout: (value) => { settings.mobileKeyboardLayout = value; },
  agentKeyboardEnabled: () => settings.mobileAgentKeyboardEnabled,
  setAgentKeyboardEnabled: (value) => { settings.mobileAgentKeyboardEnabled = value; },
  save: () => { saves++; },
  changed: () => { context.refresh(); renderKeyboard(); },
  updateIcons, tr,
});
function localize() {
  document.documentElement.lang = locale;
  document.querySelectorAll<HTMLElement>('[data-i18n]').forEach((node) => {
    node.textContent = tr(node.dataset.i18n as MessageKey);
  });
  for (const attribute of ['aria', 'title']) {
    document.querySelectorAll<HTMLElement>(`[data-i18n-${attribute}]`).forEach((node) => {
      const key = node.getAttribute(`data-i18n-${attribute}`) as MessageKey;
      node.setAttribute(attribute === 'aria' ? 'aria-label' : 'title', tr(key));
    });
  }
}
function select(id: string, refresh = true) {
  const next = panes.get(id) ?? makePane(id);
  inputs.get(activePane.id)!.hidden = true;
  activePane = next;
  inputs.get(activePane.id)!.hidden = false;
  system.restoreState();
  if (refresh) context.refresh();
}
function useNative(id = 'native-a', agent: string | null = 'codex', identity = 'process-1', refresh = true) {
  const pane = panes.get(id) ?? makePane(id);
  pane.programStatus = agent ? [{ id: identity, app: agent, state: 'working' }] : [];
  select(id, refresh);
}
function focusHerdr(id: string, refresh = true) {
  herdr.focused_pane_id = id;
  for (const pane of herdr.panes) pane.focused = pane.pane_id === id;
  for (const agent of herdr.agents) agent.focused = agent.pane_id === id;
  if (refresh) context.refresh();
}
function state() {
  const root = elements.mobileShortcuts;
  const main = root.querySelector<HTMLElement>('[data-mobile-panel="main"]')!;
  return {
    enabled: settings.mobileAgentKeyboardEnabled, saves, contextChanges,
    preset: settings.mobileKeyboardPreset, layout: settings.mobileKeyboardLayout,
    context: context.current() ?? null, paneId: activePane.id, remotePaneId: herdr.focused_pane_id,
    connectionState: activePane.connectionState,
    output: [...output], actions: [...actions], page: root.dataset.activePage,
    expanded: keyboard.isExpanded(), mainText: main.textContent,
    mainLabel: root.querySelector('[data-mobile-page="main"]')?.textContent,
    groupLabels: [...main.querySelectorAll('section')].map((node) => node.getAttribute('aria-label')),
    labels: [...main.querySelectorAll('button > span')].map((node) => node.textContent),
    activeElement: document.activeElement?.id,
    keyboardIntent: root.querySelector('[data-mobile-action="toggle-system-keyboard"]')?.getAttribute('aria-pressed'),
  };
}
localize();
renderKeyboard();
settingsController.bind();
keyboard.bind();
Object.assign(window, { qa: {
  ready: true, state, output: () => [...output], clearOutput: () => { output.length = 0; },
  refresh: renderKeyboard,
  openSettings: () => { settingsController.render(); settingsDialog.showModal(); },
  closeSettings: () => settingsDialog.close(),
  setLocale: (next: 'en' | 'zh-CN') => { locale = next; localize(); settingsController.render(); renderKeyboard(); },
  useNative, select, focusHerdr,
  herdrSnapshot: () => structuredClone(herdr),
  setHerdrSnapshotValid: (valid: boolean) => context.setHerdrSnapshotValid(valid),
  useHerdr: (focused = 'remote-a', refresh = true) => { focusHerdr(focused, false); select('herdr', refresh); },
  patchPane: (patch: Partial<TerminalPane>, refresh = true) => { Object.assign(activePane, patch); if (refresh) context.refresh(); },
  patchHerdr: (patch: Partial<HerdrBridgeState>, refresh = true) => { Object.assign(herdr, patch); if (refresh) context.refresh(); },
  setBase: (preset: MobileKeyboardPresetId, custom = false) => {
    settings.mobileKeyboardPreset = preset;
    settings.mobileKeyboardLayout = mobileKeyboardPresetLayout(preset === 'custom' ? 'default' : preset);
    if (custom) settings.mobileKeyboardLayout.pages[0].keys.push({
      id: 'fixture-custom', kind: 'text', value: 'fixture-custom\x1b!', label: 'My custom key', ariaLabel: 'My custom key',
      width: 'lg', hidden: false, repeat: false, autoEnter: false, custom: true,
    });
    settingsController.render(); renderKeyboard();
  },
  point: async (selector: string) => {
    const button = document.querySelector<HTMLElement>(selector);
    if (!button) throw new Error(`Missing fixture interaction target: ${selector}`);
    button.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' });
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const rect = button.getBoundingClientRect();
    const visible = {
      left: Math.max(0, rect.left), right: Math.min(innerWidth, rect.right),
      top: Math.max(0, rect.top), bottom: Math.min(innerHeight, rect.bottom),
    };
    // Short viewports may expose only part of a tall semantic key. Start the
    // real gesture inside the visible intersection of all scrolling ancestors.
    for (let ancestor = button.parentElement; ancestor; ancestor = ancestor.parentElement) {
      const style = getComputedStyle(ancestor);
      const clip = ancestor.getBoundingClientRect();
      if (/auto|scroll|hidden|clip/.test(style.overflowX)) {
        visible.left = Math.max(visible.left, clip.left);
        visible.right = Math.min(visible.right, clip.right);
      }
      if (/auto|scroll|hidden|clip/.test(style.overflowY)) {
        visible.top = Math.max(visible.top, clip.top);
        visible.bottom = Math.min(visible.bottom, clip.bottom);
      }
    }
    if (visible.right <= visible.left || visible.bottom <= visible.top) throw new Error(`Target has no visible area: ${selector}`);
    const point = { x: (visible.left + visible.right) / 2, y: (visible.top + visible.bottom) / 2 };
    const hit = document.elementFromPoint(point.x, point.y);
    if (!hit || !(hit === button || button.contains(hit))) throw new Error(`Occluded target: ${selector}`);
    return point;
  },
  dispose: () => keyboard.dispose(),
} });
