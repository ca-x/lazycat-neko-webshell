import assert from "node:assert/strict";
import test from "node:test";
import { enMessages } from "../i18n/messages-en.ts";
import { zhCNMessages } from "../i18n/messages-zh-cn.ts";
import { mobileAgentKeyboardGroups, mobileAgentName } from "./agent-keymap.ts";
import { normalizeMobileKeyboardLayout } from "./keyboard-layout.ts";
import { createMobileStickyState, encodeMobileShortcutInput, mobileChordInput } from "./shortcuts.ts";

const commonControl = ["ctrl-c", "ctrl-d"];
const commonEditing = ["alt-b", "alt-f", "ctrl-u", "ctrl-k", "ctrl-w", "alt-d"];

// Action order is specified independently of the profile implementation.
const expectedProfiles = {
  codex: [
    ["enter", "ctrl-j", "tab", "alt-up"],
    ["escape", "alt-down", "ctrl-bracket-right", ...commonControl],
    ["alt-left", "alt-right", "shift-tab"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-t", "ctrl-b", "ctrl-f", "alt-b", "alt-f", "ctrl-p", "ctrl-n", "ctrl-u", "ctrl-k", "ctrl-w", "alt-d", "ctrl-y", "alt-y"],
  ],
  claude: [
    ["enter", "ctrl-j", "ctrl-x-ctrl-s", "up", "tab"],
    ["escape", "ctrl-c", "ctrl-l", "ctrl-d", "ctrl-g", "ctrl-x-ctrl-e", "ctrl-b", "ctrl-x-ctrl-k"],
    ["shift-tab", "alt-p", "alt-t", "alt-o", "ctrl-t", "ctrl-s"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-o", "alt-b", "alt-f", "ctrl-p", "ctrl-n", "ctrl-u", "ctrl-k", "ctrl-w", "alt-d", "ctrl-y", "alt-y"],
  ],
  pi: [
    ["enter", "ctrl-j", "alt-enter", "alt-up"],
    ["escape", "ctrl-c", "ctrl-d", "ctrl-g"],
    ["shift-tab", "ctrl-l", "ctrl-p", "ctrl-t"],
    ["ctrl-a", "ctrl-e", "ctrl-o", "ctrl-b", "ctrl-f", ...commonEditing, "ctrl-y", "alt-y"],
  ],
  opencode: [
    ["enter", "ctrl-j", "ctrl-x-enter"],
    ["escape", ...commonControl],
    ["ctrl-x-a", "ctrl-x-m", "ctrl-p", "ctrl-a", "ctrl-x-n", "ctrl-x-l", "ctrl-x-g", "ctrl-x-c", "ctrl-x-e", "ctrl-x-t", "ctrl-x-b", "ctrl-x-s", "ctrl-x-x", "ctrl-x-y", "ctrl-x-u", "ctrl-x-r", "ctrl-r", "ctrl-x-q", "ctrl-t"],
    ["ctrl-e", "up", "down", "ctrl-b", "ctrl-f", ...commonEditing],
  ],
};

const expectedPhysicalKeys = {
  enter: ["shortcut", "Enter", "\r"],
  tab: ["shortcut", "Tab", "\x09"],
  escape: ["shortcut", "Esc", "\x1b"],
  up: ["shortcut", "Up", "\x1b[A"],
  down: ["shortcut", "Down", "\x1b[B"],
  "ctrl-j": ["chord", "Ctrl+J", "\x0a"],
  "ctrl-c": ["chord", "Ctrl+C", "\x03"],
  "ctrl-d": ["chord", "Ctrl+D", "\x04"],
  "ctrl-a": ["chord", "Ctrl+A", "\x01"],
  "ctrl-e": ["chord", "Ctrl+E", "\x05"],
  "ctrl-r": ["chord", "Ctrl+R", "\x12"],
  "ctrl-t": ["chord", "Ctrl+T", "\x14"],
  "ctrl-o": ["chord", "Ctrl+O", "\x0f"],
  "ctrl-l": ["chord", "Ctrl+L", "\x0c"],
  "ctrl-u": ["chord", "Ctrl+U", "\x15"],
  "ctrl-w": ["chord", "Ctrl+W", "\x17"],
  "ctrl-k": ["chord", "Ctrl+K", "\x0b"],
  "ctrl-y": ["chord", "Ctrl+Y", "\x19"],
  "ctrl-b": ["chord", "Ctrl+B", "\x02"],
  "ctrl-f": ["chord", "Ctrl+F", "\x06"],
  "ctrl-p": ["chord", "Ctrl+P", "\x10"],
  "ctrl-n": ["chord", "Ctrl+N", "\x0e"],
  "ctrl-g": ["chord", "Ctrl+G", "\x07"],
  "ctrl-s": ["chord", "Ctrl+S", "\x13"],
  "alt-b": ["chord", "Alt+B", "\x1bb"],
  "alt-f": ["chord", "Alt+F", "\x1bf"],
  "alt-d": ["chord", "Alt+D", "\x1bd"],
  "alt-y": ["chord", "Alt+Y", "\x1by"],
  "alt-t": ["chord", "Alt+T", "\x1bt"],
  "alt-o": ["chord", "Alt+O", "\x1bo"],
  "ctrl-bracket-right": ["chord", "Ctrl+]", "\x1d"],
  "alt-enter": ["chord", "Alt+Enter", "\x1b\r"],
  "alt-p": ["chord", "Alt+P", "\x1bp"],
  "alt-up": ["chord", "Alt+Up", "\x1b[1;3A"],
  "alt-down": ["chord", "Alt+Down", "\x1b[1;3B"],
  "alt-left": ["chord", "Alt+Left", "\x1b[1;3D"],
  "alt-right": ["chord", "Alt+Right", "\x1b[1;3C"],
  "shift-tab": ["chord", "Shift+Tab", "\x1b[Z"],
  "ctrl-x-ctrl-s": ["chord", "C-x C-s", "\x18\x13"],
  "ctrl-x-ctrl-e": ["chord", "C-x C-e", "\x18\x05"],
  "ctrl-x-ctrl-k": ["chord", "C-x C-k", "\x18\x0b"],
  "ctrl-x-enter": ["chord", "C-x Enter", "\x18\r"],
  "ctrl-x-a": ["chord", "C-x A", "\x18a"],
  "ctrl-x-m": ["chord", "C-x M", "\x18m"],
  "ctrl-x-n": ["chord", "C-x N", "\x18n"],
  "ctrl-x-l": ["chord", "C-x L", "\x18l"],
  "ctrl-x-g": ["chord", "C-x G", "\x18g"],
  "ctrl-x-c": ["chord", "C-x C", "\x18c"],
  "ctrl-x-e": ["chord", "C-x E", "\x18e"],
  "ctrl-x-t": ["chord", "C-x T", "\x18t"],
  "ctrl-x-b": ["chord", "C-x B", "\x18b"],
  "ctrl-x-s": ["chord", "C-x S", "\x18s"],
  "ctrl-x-x": ["chord", "C-x X", "\x18x"],
  "ctrl-x-y": ["chord", "C-x Y", "\x18y"],
  "ctrl-x-u": ["chord", "C-x U", "\x18u"],
  "ctrl-x-r": ["chord", "C-x R", "\x18r"],
  "ctrl-x-q": ["chord", "C-x Q", "\x18q"],
};

function entries(agent) {
  return mobileAgentKeyboardGroups(agent).flatMap((group) => group.keys);
}

function entry(agent, value) {
  const found = entries(agent).find((item) => item.key.value === value);
  assert.ok(found, `${agent}: ${value}`);
  return found;
}

for (const [agent, expected] of Object.entries(expectedProfiles)) {
  test(`${agent} keeps the four approved groups, key count and action order`, () => {
    const groups = mobileAgentKeyboardGroups(agent);
    assert.deepEqual(groups.map((group) => group.id), ["send", "control", "mode", "edit"]);
    assert.deepEqual(groups.map((group) => group.label), [
      "agentKeyboard.group.send", "agentKeyboard.group.control", "agentKeyboard.group.mode", "agentKeyboard.group.edit",
    ]);
    assert.deepEqual(groups.map((group) => group.keys.map((item) => item.key.value)), expected);
    assert.equal(entries(agent).length, { codex: 28, claude: 33, pi: 25, opencode: 36 }[agent]);
  });

  test(`${agent} sends each physical shortcut's exact bytes without implicit Enter`, () => {
    for (const { key } of entries(agent)) {
      const [kind, label, bytes] = expectedPhysicalKeys[key.value];
      assert.equal(key.kind, kind, key.value);
      assert.equal(key.label, label, key.value);
      assert.equal(key.ariaLabel, label, key.value);
      const input = key.kind === "chord"
        ? mobileChordInput(key.value)
        : encodeMobileShortcutInput(key.value, createMobileStickyState());
      assert.equal(input, bytes, `${agent}: ${key.value}`);
      assert.equal(input.includes("\r"), ["enter", "alt-enter", "ctrl-x-enter"].includes(key.value), key.value);
    }
  });

  test(`${agent} uses supported keys with no repeat, automatic Enter or customization`, () => {
    const keys = entries(agent).map((item) => item.key);
    for (const key of keys) {
      assert.equal(key.repeat, false, key.value);
      assert.equal(key.autoEnter, false, key.value);
      assert.equal(key.hidden, false, key.value);
      assert.equal(key.custom, false, key.value);
      assert.equal(key.width, "md", key.value);
    }
    const supported = (key) => [key.id, key.kind, key.value, key.label];
    for (const key of keys) {
      const normalized = normalizeMobileKeyboardLayout({ pages: [{ id: "main", keys: [key] }] }).pages[0].keys[0];
      assert.deepEqual(supported(normalized), supported(key));
    }
  });
}

test("Agent identities have stable names and distinct key IDs", () => {
  assert.deepEqual(Object.keys(expectedProfiles).map(mobileAgentName), ["Codex CLI", "Claude Code", "Pi", "OpenCode"]);
  const allIds = Object.keys(expectedProfiles).flatMap((agent) => entries(agent).map((item) => item.key.id));
  assert.equal(new Set(allIds).size, allIds.length);
  for (const agent of Object.keys(expectedProfiles)) {
    assert.deepEqual(mobileAgentKeyboardGroups(agent), mobileAgentKeyboardGroups(agent));
  }
});

test("context-dependent actions retain visible conditions without inferring application state", () => {
  const expected = [
    ["codex", "enter", "send", "codexSend"],
    ["codex", "tab", "queue", "codexQueue"],
    ["codex", "alt-up", "retrieveEdit", "codexRetrieve"],
    ["codex", "alt-down", "towardInput", "codexNavigate"],
    ["codex", "ctrl-bracket-right", "skipQuestion", "codexSkip"],
    ["claude", "enter", "send", "claudeSend"],
    ["claude", "ctrl-x-ctrl-s", "sendQueuedNow", "queuedMessages"],
    ["claude", "up", "retrieve", "claudeRetrieve"],
    ["pi", "enter", "send", "piSend"],
    ["pi", "alt-enter", "followUp", "piFollowUp"],
    ["pi", "alt-up", "retrieve", "queuedMessages"],
  ];
  for (const [agent, value, action, condition] of expected) {
    const item = entry(agent, value);
    assert.equal(item.label, `agentKeyboard.action.${action}`, `${agent}: ${value}`);
    assert.equal(item.condition, `agentKeyboard.condition.${condition}`, `${agent}: ${value}`);
  }
  for (const locale of [enMessages, zhCNMessages]) {
    assert.match(locale["agentKeyboard.condition.codexRetrieve"], /async|异步/i);
    assert.match(locale["agentKeyboard.condition.codexSkip"], /permission|权限/);
    assert.match(locale["agentKeyboard.condition.claudeRetrieve"], /first line|首行/i);
    assert.match(locale["agentKeyboard.condition.piSend"], /tool|工具/);
    assert.match(locale["agentKeyboard.condition.piFollowUp"], /task|任务/);
  }
});

test("all group, action, condition and local-UI messages exist in both locales", () => {
  const required = new Set([
    "setting.mobileAgentKeyboardEnabled", "setting.mobileAgentKeyboardHelp",
  ]);
  for (const agent of Object.keys(expectedProfiles)) {
    for (const group of mobileAgentKeyboardGroups(agent)) {
      assert.ok(group.label.startsWith("agentKeyboard."));
      required.add(group.label);
      for (const item of group.keys) {
        assert.ok(item.label.startsWith("agentKeyboard."));
        required.add(item.label);
        if (item.condition) {
          assert.ok(item.condition.startsWith("agentKeyboard."));
          required.add(item.condition);
        }
      }
    }
  }
  for (const key of Object.keys(enMessages).filter((key) => key.startsWith("agentKeyboard."))) required.add(key);
  assert.deepEqual(
    Object.keys(enMessages).filter((key) => key.startsWith("agentKeyboard.")).sort(),
    Object.keys(zhCNMessages).filter((key) => key.startsWith("agentKeyboard.")).sort(),
  );
  for (const locale of [enMessages, zhCNMessages]) {
    for (const key of required) {
      assert.ok(Object.hasOwn(locale, key), key);
      assert.ok(locale[key].trim().length, key);
    }
  }
});
