import assert from "node:assert/strict";
import test from "node:test";
import { mobileChordInput } from "./shortcuts.ts";
import { mobileKeyboardPresetLayout, normalizeMobileKeyboardLayout, normalizeMobileKeyboardPreset } from "./keyboard-layout.ts";

test("Readline and Emacs chords send exact control bytes without an Enter", () => {
  const expected = {
    "ctrl-a": "\x01", "ctrl-e": "\x05", "ctrl-r": "\x12", "ctrl-u": "\x15", "ctrl-w": "\x17",
    "ctrl-k": "\x0b", "ctrl-l": "\x0c", "ctrl-space": "\x00", "ctrl-x-ctrl-s": "\x18\x13",
    "ctrl-x-ctrl-f": "\x18\x06", "alt-b": "\x1bb", "alt-f": "\x1bf", "alt-x": "\x1bx", "alt-period": "\x1b.",
    "ctrl-c": "\x03", "ctrl-z": "\x1a", "shift-tab": "\x1b[Z",
  };
  for (const [chord, bytes] of Object.entries(expected)) assert.equal(mobileChordInput(chord), bytes);
  for (const invalid of ["__proto__", "constructor", "ctrl-unknown"]) assert.equal(mobileChordInput(invalid), undefined);
});

test("editor presets survive settings normalization and custom layouts retain old keys", () => {
  for (const preset of ["vim", "emacs"]) {
    assert.equal(normalizeMobileKeyboardPreset(preset), preset);
    const layout = mobileKeyboardPresetLayout(preset);
    assert.deepEqual(normalizeMobileKeyboardLayout(layout), layout);
    assert.ok(layout.pages[0].keys.every((key) => !key.autoEnter && !key.repeat));
  }
  assert.ok(mobileKeyboardPresetLayout("vim").pages[0].keys.some((key) => key.value === ":"));
  assert.ok(mobileKeyboardPresetLayout("emacs").pages[0].keys.some((key) => key.value === "ctrl-x-ctrl-s"));
  const saved = { pages: [{ id: "main", keys: [{ id: "main-menu", kind: "action", value: "pane-menu", label: "My menu", hidden: true, width: "lg" }] }] };
  const restored = normalizeMobileKeyboardLayout(saved).pages[0].keys;
  assert.equal(restored.length, 1);
  assert.equal(restored[0].label, "My menu");
  assert.equal(restored[0].hidden, true);
});
