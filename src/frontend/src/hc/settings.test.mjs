import assert from "node:assert/strict";
import test from "node:test";
import { normalizeHcHistoryLines } from "./settings.ts";

test("HC history is an integer bounded to 0..10000 with default 1000", () => {
  for (const value of [undefined, null, true, "", "invalid", NaN, Infinity]) {
    assert.equal(normalizeHcHistoryLines(value), 1000);
  }
  for (const [value, expected] of [[0, 0], [1000, 1000], [10000, 10000], [-1, 0], [10001, 10000], [12.9, 12], ["42", 42]]) {
    assert.equal(normalizeHcHistoryLines(value), expected);
  }
});

test("HC history settings view uses an integer field and follows advertised HC availability", async () => {
  const { renderHcHistorySettings, syncHcHistorySettings } = await import("./settings-view.ts");
  const html = renderHcHistorySettings();
  assert.match(html, /min="0" max="10000" step="1"/);
  assert.match(html, /data-i18n="field.hcHistoryLines"/);
  assert.match(html, /data-i18n="setting.hcHistoryLinesHelp"/);
  const elements = { hcHistorySettings: { hidden: false }, hcHistoryLines: { value: "" } };
  syncHcHistorySettings(elements, { hcHistoryLines: 0 }, true);
  assert.equal(elements.hcHistorySettings.hidden, false);
  assert.equal(elements.hcHistoryLines.value, "0");
  syncHcHistorySettings(elements, { hcHistoryLines: 1000 }, false);
  assert.equal(elements.hcHistorySettings.hidden, true);
});

test("restored HC panes keep history settings visible when HC becomes unavailable", async () => {
  const { hcHistorySettingsVisible } = await import("./settings-view.ts");
  const restored = [{ sessionBackend: "hc", selector: "box@owner" }];
  assert.equal(hcHistorySettingsVisible(false, restored, "box@owner"), true);
  assert.equal(hcHistorySettingsVisible(false, restored, "other@owner"), false);
  assert.equal(hcHistorySettingsVisible(false, [{ sessionBackend: "webshell", selector: "box@owner" }], "box@owner"), false);
  assert.equal(hcHistorySettingsVisible(true, [], "box@owner"), true);
});
