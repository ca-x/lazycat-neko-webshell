import assert from "node:assert/strict";
import test from "node:test";
import { enMessages } from "../i18n/messages-en.ts";
import { zhCNMessages } from "../i18n/messages-zh-cn.ts";
import { mobileAgentKeyboardGroups, mobileAgentName } from "./agent-keymap.ts";
import { normalizeMobileKeyboardLayout } from "./keyboard-layout.ts";
import { createMobileStickyState, encodeMobileShortcutInput, mobileChordInput } from "./shortcuts.ts";

// Approved action order, independently specified rather than read from the chord table.
const expectedProfiles = {
  codex: [
    ["enter", "ctrl-j", "tab", "alt-up"],
    ["escape", "alt-down", "ctrl-bracket-right"],
    ["alt-left", "alt-right", "shift-tab"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-t"],
  ],
  claude: [
    ["enter", "ctrl-j", "ctrl-x-ctrl-s", "up"],
    ["escape"],
    ["shift-tab", "alt-p"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-o"],
  ],
  pi: [
    ["enter", "ctrl-j", "alt-enter", "alt-up"],
    ["escape"],
    ["shift-tab", "ctrl-l"],
    ["ctrl-a", "ctrl-e", "ctrl-o"],
  ],
  opencode: [
    ["enter", "ctrl-j", "ctrl-x-enter"],
    ["escape"],
    ["ctrl-x-a", "ctrl-x-m"],
    ["ctrl-a", "ctrl-e", "up", "down"],
  ],
};

const expectedPhysicalKeys = {
  enter: ["shortcut", "Enter", "\r"],
  tab: ["shortcut", "Tab", "\x09"],
  escape: ["shortcut", "Esc", "\x1b"],
  up: ["shortcut", "Up", "\x1b[A"],
  down: ["shortcut", "Down", "\x1b[B"],
  "ctrl-j": ["chord", "Ctrl+J", "\x0a"],
  "ctrl-a": ["chord", "Ctrl+A", "\x01"],
  "ctrl-e": ["chord", "Ctrl+E", "\x05"],
  "ctrl-r": ["chord", "Ctrl+R", "\x12"],
  "ctrl-t": ["chord", "Ctrl+T", "\x14"],
  "ctrl-o": ["chord", "Ctrl+O", "\x0f"],
  "ctrl-l": ["chord", "Ctrl+L", "\x0c"],
  "ctrl-bracket-right": ["chord", "Ctrl+]", "\x1d"],
  "alt-enter": ["chord", "Alt+Enter", "\x1b\r"],
  "alt-p": ["chord", "Alt+P", "\x1bp"],
  "alt-up": ["chord", "Alt+Up", "\x1b[1;3A"],
  "alt-down": ["chord", "Alt+Down", "\x1b[1;3B"],
  "alt-left": ["chord", "Alt+Left", "\x1b[1;3D"],
  "alt-right": ["chord", "Alt+Right", "\x1b[1;3C"],
  "shift-tab": ["chord", "Shift+Tab", "\x1b[Z"],
  "ctrl-x-ctrl-s": ["chord", "C-x C-s", "\x18\x13"],
  "ctrl-x-enter": ["chord", "C-x Enter", "\x18\r"],
  "ctrl-x-a": ["chord", "C-x A", "\x18a"],
  "ctrl-x-m": ["chord", "C-x M", "\x18m"],
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
    assert.equal(entries(agent).length, { codex: 14, claude: 11, pi: 10, opencode: 10 }[agent]);
    const limits = [[2, 4], [1, 3], [2, 3], [2, 4]];
    groups.forEach((group, index) => {
      assert.ok(group.keys.length >= limits[index][0] && group.keys.length <= limits[index][1]);
    });
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
    const normalized = normalizeMobileKeyboardLayout({ pages: [{ id: "main", keys }] }).pages[0].keys;
    const supported = (key) => [key.id, key.kind, key.value, key.label];
    assert.deepEqual(normalized.map(supported), keys.map(supported));
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
