import assert from "node:assert/strict";
import test from "node:test";

import { createMobileKeyboardLayoutSettingsController } from "./keyboard-layout-settings-controller.ts";
import { mobileKeyboardPresetLayout } from "./keyboard-layout.ts";
import { renderMobileKeyboardLayoutSettingsView } from "./settings/keyboard-layout-view.ts";

function setup({ preset = "custom", enabled = false, withOptions = true } = {}) {
  const checkbox = Object.assign(new EventTarget(), { checked: false });
  const presetControl = Object.assign(new EventTarget(), { value: preset });
  const undo = Object.assign(new EventTarget(), { disabled: true, hidden: true });
  const draftLabel = { value: "Unsaved custom key" };
  const controls = new Map([
    ["[data-mobile-agent-keyboard-enabled]", checkbox],
    ["[data-mobile-layout-preset]", presetControl],
    ["[data-mobile-layout-undo]", undo],
    ["[data-mobile-key-label]", draftLabel],
  ]);
  const root = Object.assign(new EventTarget(), { querySelector: selector => controls.get(selector) ?? null });
  const layout = mobileKeyboardPresetLayout("vim");
  layout.pages[0].keys[0].hidden = true;
  const state = { preset, layout, enabled };
  const calls = [];
  const controller = createMobileKeyboardLayoutSettingsController({
    root,
    preset: () => state.preset,
    layout: () => state.layout,
    setPreset: value => { calls.push("preset"); state.preset = value; },
    setLayout: value => { calls.push("layout"); state.layout = value; },
    ...(withOptions ? {
      agentKeyboardEnabled: () => state.enabled,
      setAgentKeyboardEnabled: value => { calls.push("enabled"); state.enabled = value; },
    } : {}),
    save: () => calls.push("save"),
    changed: () => calls.push("changed"),
    updateIcons: () => {},
    tr: key => key,
  });
  controller.bind();
  return { controller, checkbox, presetControl, undo, draftLabel, state, calls };
}

test("opt-in remains accessible outside the collapsed layout editor with linked localized help", () => {
  const html = renderMobileKeyboardLayoutSettingsView();
  const collapsedEditor = html.indexOf('<details class="mobile-keyboard-customize"');
  const visible = html.slice(0, collapsedEditor);
  assert.ok(collapsedEditor > 0);
  assert.match(visible, /type="checkbox" data-mobile-agent-keyboard-enabled/);
  assert.doesNotMatch(visible, /\bchecked\b/);
  assert.match(visible, /data-i18n="setting\.mobileAgentKeyboardEnabled"/);
  assert.match(visible, /aria-describedby="mobileAgentKeyboardHelp"/);
  assert.match(visible, /id="mobileAgentKeyboardHelp" data-i18n="setting\.mobileAgentKeyboardHelp"/);
  assert.match(visible, /Shortcuts may vary with Agent version or custom bindings/);
  assert.doesNotMatch(html.slice(collapsedEditor, html.indexOf(">", collapsedEditor)), /\bopen\b/);
});

test("render syncs the opt-in in both directions and defaults to off when options are omitted", () => {
  const s = setup({ enabled: true });
  assert.equal(s.checkbox.checked, true);
  s.state.enabled = false;
  s.controller.render();
  assert.equal(s.checkbox.checked, false);
  assert.deepEqual(s.calls, []);

  const legacy = setup({ enabled: true, withOptions: false });
  assert.equal(legacy.checkbox.checked, false);
  legacy.checkbox.checked = true;
  legacy.checkbox.dispatchEvent(new Event("change"));
  assert.deepEqual(legacy.calls, []);
});

test("toggling saves and notifies without changing any preset, custom layout, draft or undo history", () => {
  for (const preset of ["default", "operations", "editor", "vim", "emacs", "custom"]) {
    const s = setup({ preset });
    const layout = s.state.layout;
    const original = structuredClone(layout);
    for (const enabled of [true, false]) {
      s.calls.length = 0;
      s.checkbox.checked = enabled;
      s.checkbox.dispatchEvent(new Event("change"));
      assert.equal(s.state.enabled, enabled);
      assert.deepEqual(s.calls, ["enabled", "save", "changed"]);
      assert.equal(s.state.preset, preset);
      assert.equal(s.state.layout, layout);
      assert.deepEqual(layout, original);
      assert.equal(s.draftLabel.value, "Unsaved custom key");
      assert.equal(s.undo.disabled, true);
      assert.equal(s.undo.hidden, true);
    }
  }
});

test("a toggle preserves existing layout undo and layout undo preserves the opt-in", () => {
  const s = setup();
  const original = structuredClone(s.state.layout);
  s.presetControl.value = "emacs";
  s.presetControl.dispatchEvent(new Event("change"));
  assert.equal(s.undo.disabled, false);
  s.checkbox.checked = true;
  s.checkbox.dispatchEvent(new Event("change"));
  s.undo.dispatchEvent(new Event("click"));
  assert.equal(s.state.preset, "custom");
  assert.deepEqual(s.state.layout, original);
  assert.equal(s.state.enabled, true);
  assert.equal(s.checkbox.checked, true);
  assert.equal(s.undo.hidden, true);
});
