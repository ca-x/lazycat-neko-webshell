import assert from "node:assert/strict";
import test from "node:test";
import { renderMobileKeyboardView } from "./keyboard-view.ts";

test("keeps terminal keys ahead of two persistent keyboard controls", () => {
  const html = renderMobileKeyboardView();
  const toolbar = html.slice(html.indexOf('class="mobile-keyboard-toolbar"'), html.indexOf('class="mobile-expanded-keyboard"'));
  assert.match(html, /data-active-page="main" data-expanded="false"/);
  for (const key of ["escape", "tab", "ctrl", "left", "down", "up", "right", "alt", "shift"]) {
    assert.ok(toolbar.includes(`data-mobile-shortcut="${key}"`));
  }
  const actions = toolbar.slice(toolbar.indexOf('class="mobile-keyboard-toolbar-actions"'));
  assert.equal((actions.match(/<button/g) || []).length, 2);
  assert.match(actions, /toggle-system-keyboard/);
  assert.match(actions, /toggle-expanded-keyboard/);
  assert.equal((html.match(/data-mobile-action="secret-file"/g) || []).length, 1);
  assert.equal((html.match(/data-mobile-action="compose-text"/g) || []).length, 1);
});

test("exposes one level of pages with a single editable function key set", () => {
  const html = renderMobileKeyboardView();
  const pages = [...html.matchAll(/data-mobile-page="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(pages, ["main", "nav", "fn", "sym", "typing", "ops"]);
  assert.doesNotMatch(html, /mobile-keyboard-mode-tabs|data-mobile-mode-tab|data-mobile-panel="function"/);
  assert.match(html, /data-mobile-page="main" aria-pressed="true"/);
  assert.match(html, /data-mobile-panel="typing"[^>]*hidden/);
  assert.match(html, /id="mobileExpandedKeyboard" hidden/);
});
