import assert from "node:assert/strict";
import test from "node:test";
import { mobileAgentKeyboardGroups } from "./agent-keymap.ts";
import { loadAgentKeyboardUsage, rankAgentKeyboardKeys, recordAgentKeyboardUse } from "./agent-keyboard-usage.ts";

test("server usage promotes only the pressed key within its Agent and group", async () => {
  const calls = [];
  globalThis.fetch = async (_url, init) => {
    calls.push(init?.method ?? "GET");
    return { ok: true, json: async () => init?.method === "POST"
      ? { agent: "codex", group: "send", shortcut: "shift-left", count: 1 }
      : { userId: "alice", usage: [{ agent: "codex", group: "send", shortcut: "shift-left", count: 12 }] } };
  };
  await loadAgentKeyboardUsage();
  const codex = mobileAgentKeyboardGroups("codex");
  const send = codex.find((group) => group.id === "send");
  const control = codex.find((group) => group.id === "control");
  assert.ok(send && control);
  const original = send.keys.map((entry) => entry.key.value);
  recordAgentKeyboardUse("codex", "send", "shift-left");
  assert.equal(rankAgentKeyboardKeys("codex", "send", send.keys)[0].key.value, "shift-left");
  assert.deepEqual(send.keys.map((entry) => entry.key.value), original);
  assert.equal(rankAgentKeyboardKeys("claude", "send", mobileAgentKeyboardGroups("claude")[0].keys)[0].key.value, "enter");
  assert.equal(rankAgentKeyboardKeys("codex", "control", control.keys)[0].key.value, "escape");
  assert.deepEqual(calls, ["GET", "POST"]);
});

test("network failure never blocks a shortcut", () => {
  globalThis.fetch = () => Promise.reject(new Error("offline"));
  assert.doesNotThrow(() => recordAgentKeyboardUse("pi", "edit", "alt-b"));
});

test("a later account snapshot replaces old rankings and retries after failure", async () => {
  globalThis.fetch = () => Promise.reject(new Error("offline"));
  assert.equal(await loadAgentKeyboardUsage(), false);
  globalThis.fetch = async () => ({ ok: true, json: async () => ({ userId: "bob", usage: [
    { agent: "claude", group: "send", shortcut: "shift-enter", count: 9 },
  ] }) });
  assert.equal(await loadAgentKeyboardUsage(), true);
  const claude = mobileAgentKeyboardGroups("claude")[0];
  assert.equal(rankAgentKeyboardKeys("claude", claude.id, claude.keys)[0].key.value, "shift-enter");
  const codex = mobileAgentKeyboardGroups("codex")[0];
  assert.equal(rankAgentKeyboardKeys("codex", codex.id, codex.keys)[0].key.value, "enter");
});
