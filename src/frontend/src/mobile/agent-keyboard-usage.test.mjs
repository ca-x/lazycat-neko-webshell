import assert from "node:assert/strict";
import test from "node:test";
import { mobileAgentKeyboardGroups } from "./agent-keymap.ts";
import { rankAgentKeyboardKeys, recordAgentKeyboardUse } from "./agent-keyboard-usage.ts";

test("Agent shortcut use promotes only the pressed key within its Agent and group", () => {
  const data = new Map();
  globalThis.window = { localStorage: {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => data.set(key, value),
  } };
  const codex = mobileAgentKeyboardGroups("codex");
  const send = codex.find((group) => group.id === "send");
  const control = codex.find((group) => group.id === "control");
  assert.ok(send && control);
  const original = send.keys.map((entry) => entry.key.value);
  recordAgentKeyboardUse("codex", "send", "shift-left");
  assert.equal(rankAgentKeyboardKeys("codex", "send", send.keys)[0].key.value, "shift-left");
  assert.deepEqual(send.keys.map((entry) => entry.key.value), original, "profile defaults stay immutable");
  assert.equal(rankAgentKeyboardKeys("claude", "send", mobileAgentKeyboardGroups("claude")[0].keys)[0].key.value, "enter");
  assert.equal(rankAgentKeyboardKeys("codex", "control", control.keys)[0].key.value, "escape");
});

test("Agent shortcut ranking tolerates disabled or malformed local storage", () => {
  globalThis.window = { localStorage: {
    getItem: () => "{not json",
    setItem: () => { throw new Error("storage unavailable"); },
  } };
  const keys = mobileAgentKeyboardGroups("codex")[0].keys;
  assert.deepEqual(rankAgentKeyboardKeys("codex", "send", keys), keys);
  assert.doesNotThrow(() => recordAgentKeyboardUse("codex", "send", "enter"));
});
