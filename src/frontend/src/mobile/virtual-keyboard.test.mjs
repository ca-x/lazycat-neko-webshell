import assert from "node:assert/strict";
import test from "node:test";
import { encodeMobileShortcutKeyInput } from "../keyboard.ts";
import { renderMobileTypingKeyboard } from "./virtual-keyboard.ts";
import { mobileKeyboardPresetLayout, normalizeMobileKeyboardLayout } from "./keyboard-layout.ts";
import { renderMobileSymbolKeyboardPanel } from "./quick-input.ts";

test("optional ABC page supplies QWERTY without a second function keyboard", () => {
  const html = renderMobileTypingKeyboard();
  assert.equal(html.match(/data-mobile-panel=/g)?.length, 1);
  assert.match(html, /data-mobile-panel="typing"[^>]*hidden/);
  assert.equal(html.match(/data-mobile-virtual-row=/g)?.length, 6);
  for (const value of "qwertyuiopasdfghjklzxcvbnm1234567890") {
    assert.ok(html.includes(`data-mobile-shortcut="${value}"`));
    assert.notEqual(encodeMobileShortcutKeyInput(value), undefined);
  }
  for (const control of ["shift", "ctrl", "alt", "tab", "enter", "backspace", "left", "up", "down", "right"]) {
    assert.ok(html.includes(`data-mobile-shortcut="${control}"`));
  }
  assert.match(html, /data-mobile-shortcut=" " aria-label="Space"/);
  assert.doesNotMatch(html, /data-mobile-panel="function"|data-mobile-shortcut="f12"/);
});

test("sticky Shift supports editor capitals and shifted shell symbols", () => {
  assert.equal(encodeMobileShortcutKeyInput("q", { shift: true }), "Q");
  assert.equal(encodeMobileShortcutKeyInput("1", { shift: true }), "!");
  assert.equal(encodeMobileShortcutKeyInput("/", { shift: true }), "?");
  assert.equal(encodeMobileShortcutKeyInput("q", { ctrl: true, shift: true }), "\x11");
});

test("ABC and symbols retain every printable ASCII character after flattening", () => {
  const html = renderMobileTypingKeyboard();
  const values = [...html.matchAll(/data-mobile-shortcut="([^"]*)"/g)].map((match) => match[1].replaceAll("&#039;", "'").replaceAll("&quot;", '"').replaceAll("&amp;", "&"));
  const symbols = mobileKeyboardPresetLayout("default").pages.find((page) => page.id === "sym").keys;
  const legacy = normalizeMobileKeyboardLayout({ pages: [{ id: "sym", keys: symbols.slice(0, 18) }] });
  assert.equal(legacy.pages.find((page) => page.id === "sym").keys.length, 18);
  for (const keys of [symbols, legacy.pages.find((page) => page.id === "sym").keys]) {
    const produced = new Set();
    for (const value of [...values, ...keys.map((key) => key.value)]) {
      for (const shift of [false, true]) {
        const input = encodeMobileShortcutKeyInput(value, { shift });
        if (input?.length === 1) produced.add(input);
      }
    }
    for (let code = 32; code <= 126; code += 1) assert.ok(produced.has(String.fromCharCode(code)), `missing character: ${String.fromCharCode(code)}`);
  }
  for (const symbol of [".", ",", "=", "&#039;"]) assert.ok(renderMobileSymbolKeyboardPanel("default").includes(`data-mobile-shortcut="${symbol}"`));
});
