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
    ["enter", "ctrl-j", "shift-enter", "alt-enter", "tab", "alt-up", "shift-left"],
    ["escape", "alt-down", "shift-right", "ctrl-bracket-right", "alt-comma", "shift-down", "alt-period", "shift-up", "escape-escape", "f8", "ctrl-x", ...commonControl],
    ["alt-left", "alt-right", "shift-tab", "alt-r", "ctrl-slash"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-t", "ctrl-g", "ctrl-o", "ctrl-l", "ctrl-s", "pageUp", "pageDown", "home", "end", "ctrl-home", "ctrl-end", "shift-home", "shift-end", "ctrl-b", "ctrl-f", "alt-b", "alt-f", "ctrl-p", "ctrl-n", "ctrl-u", "ctrl-k", "ctrl-w", "alt-d", "ctrl-y", "alt-y"],
  ],
  claude: [
    ["enter", "ctrl-j", "shift-enter", "ctrl-x-ctrl-s", "ctrl-enter", "ctrl-x-enter", "up", "tab"],
    ["escape", "escape-escape", "ctrl-c", "ctrl-l", "ctrl-d", "ctrl-g", "ctrl-x-ctrl-e", "ctrl-b", "ctrl-x-ctrl-b", "ctrl-x-ctrl-k", "ctrl-z"],
    ["shift-tab", "alt-p", "alt-t", "alt-o", "ctrl-t", "ctrl-s"],
    ["ctrl-a", "ctrl-e", "ctrl-r", "ctrl-o", "ctrl-minus", "down", "pageUp", "pageDown", "shift-left", "shift-right", "shift-up", "shift-down", "alt-b", "alt-f", "ctrl-p", "ctrl-n", "ctrl-u", "ctrl-k", "ctrl-w", "alt-d", "ctrl-y", "alt-y"],
  ],
  pi: [
    ["enter", "ctrl-j", "shift-enter", "alt-enter", "alt-up"],
    ["escape", "ctrl-c", "ctrl-d", "ctrl-g", "ctrl-z"],
    ["shift-tab", "ctrl-l", "ctrl-p", "ctrl-shift-p", "ctrl-t", "ctrl-x"],
    ["ctrl-a", "ctrl-e", "ctrl-o", "ctrl-minus", "up", "down", "pageUp", "pageDown", "ctrl-shift-up", "ctrl-shift-down", "ctrl-home", "ctrl-end", "ctrl-b", "ctrl-f", ...commonEditing, "ctrl-y", "alt-y"],
  ],
  opencode: [
    ["enter", "ctrl-j", "shift-enter", "alt-enter", "ctrl-x-enter"],
    ["escape", ...commonControl, "ctrl-z"],
    ["ctrl-x-a", "ctrl-x-m", "shift-tab", "f2", "shift-f2", "ctrl-p", "ctrl-a", "ctrl-x-n", "ctrl-x-l", "ctrl-o", "ctrl-x-g", "ctrl-x-i", "ctrl-x-w", "ctrl-shift-t", "ctrl-x-1", "ctrl-x-2", "ctrl-x-3", "ctrl-x-4", "ctrl-x-5", "ctrl-x-6", "ctrl-x-7", "ctrl-x-8", "ctrl-x-9", "ctrl-x-0", "ctrl-x-c", "ctrl-x-e", "ctrl-x-t", "ctrl-x-down", "ctrl-x-up", "ctrl-x-left", "ctrl-x-right", "ctrl-x-b", "ctrl-x-s", "ctrl-x-x", "ctrl-x-y", "ctrl-x-u", "ctrl-x-r", "ctrl-r", "ctrl-x-q", "ctrl-t", "ctrl-g", "alt-up", "alt-down", "ctrl-shift-tab"],
    ["ctrl-e", "up", "down", "pageUp", "pageDown", "home", "end", "alt-home", "alt-end", "shift-left", "shift-right", "shift-up", "shift-down", "ctrl-alt-b", "ctrl-alt-f", "ctrl-alt-y", "ctrl-alt-e", "ctrl-alt-u", "ctrl-alt-d", "ctrl-alt-g", "ctrl-b", "ctrl-f", ...commonEditing],
  ],
};

const expectedPhysicalKeys = {
  enter: ["shortcut", "Enter", "\r"],
  tab: ["shortcut", "Tab", "\x09"],
  escape: ["shortcut", "Esc", "\x1b"],
  up: ["shortcut", "Up", "\x1b[A"],
  down: ["shortcut", "Down", "\x1b[B"],
  home: ["shortcut", "Home", "\x1b[H"],
  end: ["shortcut", "End", "\x1b[F"],
  pageUp: ["shortcut", "PgUp", "\x1b[5~"],
  pageDown: ["shortcut", "PgDn", "\x1b[6~"],
  f2: ["shortcut", "F2", "\x1bOQ"],
  f8: ["shortcut", "F8", "\x1b[19~"],
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
  "ctrl-x": ["chord", "Ctrl+X", "\x18"],
  "ctrl-z": ["chord", "Ctrl+Z", "\x1a"],
  "ctrl-minus": ["chord", "Ctrl+-", "\x1f"],
  "ctrl-slash": ["chord", "Ctrl+/", "\x1f"],
  "ctrl-enter": ["chord", "Ctrl+Enter", "\x1b[13;5u"],
  "ctrl-shift-p": ["chord", "Ctrl+Shift+P", "\x1b[112;6u"],
  "ctrl-shift-t": ["chord", "Ctrl+Shift+T", "\x1b[116;6u"],
  "ctrl-shift-tab": ["chord", "Ctrl+Shift+Tab", "\x1b[9;6u"],
  "ctrl-shift-up": ["chord", "Ctrl+Shift+Up", "\x1b[1;6A"],
  "ctrl-shift-down": ["chord", "Ctrl+Shift+Down", "\x1b[1;6B"],
  "ctrl-alt-b": ["chord", "Ctrl+Alt+B", "\x1b\x02"],
  "ctrl-alt-f": ["chord", "Ctrl+Alt+F", "\x1b\x06"],
  "alt-b": ["chord", "Alt+B", "\x1bb"],
  "alt-f": ["chord", "Alt+F", "\x1bf"],
  "alt-d": ["chord", "Alt+D", "\x1bd"],
  "alt-y": ["chord", "Alt+Y", "\x1by"],
  "alt-t": ["chord", "Alt+T", "\x1bt"],
  "alt-o": ["chord", "Alt+O", "\x1bo"],
  "ctrl-bracket-right": ["chord", "Ctrl+]", "\x1d"],
  "alt-enter": ["chord", "Alt+Enter", "\x1b\r"],
  "alt-p": ["chord", "Alt+P", "\x1bp"],
  "alt-r": ["chord", "Alt+R", "\x1br"],
  "alt-home": ["chord", "Alt+Home", "\x1b[1;3H"],
  "alt-end": ["chord", "Alt+End", "\x1b[1;3F"],
  "shift-enter": ["chord", "Shift+Enter", "\x1b[13;2u"],
  "ctrl-home": ["chord", "Ctrl+Home", "\x1b[1;5H"],
  "ctrl-end": ["chord", "Ctrl+End", "\x1b[1;5F"],
  "shift-home": ["chord", "Shift+Home", "\x1b[1;2H"],
  "shift-end": ["chord", "Shift+End", "\x1b[1;2F"],
  "ctrl-alt-y": ["chord", "Ctrl+Alt+Y", "\x1b\x19"],
  "ctrl-alt-e": ["chord", "Ctrl+Alt+E", "\x1b\x05"],
  "ctrl-alt-u": ["chord", "Ctrl+Alt+U", "\x1b\x15"],
  "ctrl-alt-d": ["chord", "Ctrl+Alt+D", "\x1b\x04"],
  "ctrl-alt-g": ["chord", "Ctrl+Alt+G", "\x1b\x07"],
  "alt-comma": ["chord", "Alt+,", "\x1b,"],
  "alt-period": ["chord", "Alt+.", "\x1b."],
  "alt-up": ["chord", "Alt+Up", "\x1b[1;3A"],
  "alt-down": ["chord", "Alt+Down", "\x1b[1;3B"],
  "alt-left": ["chord", "Alt+Left", "\x1b[1;3D"],
  "alt-right": ["chord", "Alt+Right", "\x1b[1;3C"],
  "shift-tab": ["chord", "Shift+Tab", "\x1b[Z"],
  "shift-up": ["chord", "Shift+Up", "\x1b[1;2A"],
  "shift-down": ["chord", "Shift+Down", "\x1b[1;2B"],
  "shift-right": ["chord", "Shift+Right", "\x1b[1;2C"],
  "shift-left": ["chord", "Shift+Left", "\x1b[1;2D"],
  "escape-escape": ["chord", "Esc Esc", "\x1b\x1b"],
  "shift-f2": ["chord", "Shift+F2", "\x1b[1;2Q"],
  "ctrl-x-left": ["chord", "C-x Left", "\x18\x1b[D"],
  "ctrl-x-right": ["chord", "C-x Right", "\x18\x1b[C"],
  "ctrl-x-up": ["chord", "C-x Up", "\x18\x1b[A"],
  "ctrl-x-down": ["chord", "C-x Down", "\x18\x1b[B"],
  "ctrl-x-ctrl-s": ["chord", "C-x C-s", "\x18\x13"],
  "ctrl-x-ctrl-e": ["chord", "C-x C-e", "\x18\x05"],
  "ctrl-x-ctrl-k": ["chord", "C-x C-k", "\x18\x0b"],
  "ctrl-x-ctrl-b": ["chord", "C-x C-b", "\x18\x02"],
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
  "ctrl-x-i": ["chord", "C-x I", "\x18i"],
  "ctrl-x-w": ["chord", "C-x W", "\x18w"],
  ...Object.fromEntries(["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].map((number) => [`ctrl-x-${number}`, ["chord", `C-x ${number}`, `\x18${number}`]])),
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
    assert.equal(entries(agent).length, { codex: 53, claude: 47, pi: 38, opencode: 81 }[agent]);
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
