import type { MessageKey } from "../i18n/messages-en.ts";
import type { MobileKeyboardKey } from "./keyboard-layout-types.ts";
import { MOBILE_CHORDS, type MobileChord } from "./key-chords.ts";

export type MobileKeyboardAgent = "claude" | "codex" | "opencode" | "pi";

export type MobileAgentKeyboardEntry = {
  key: MobileKeyboardKey;
  label: MessageKey;
  condition?: MessageKey;
};

export type MobileAgentKeyboardGroup = {
  id: "send" | "control" | "mode" | "edit";
  label: MessageKey;
  keys: readonly MobileAgentKeyboardEntry[];
};

const SHORTCUT_LABELS = {
  enter: "Enter", escape: "Esc", tab: "Tab", up: "Up", down: "Down", left: "Left", right: "Right",
  home: "Home", end: "End", pageUp: "PgUp", pageDown: "PgDn", f2: "F2", f8: "F8",
} as const;

function entry(
  agent: MobileKeyboardAgent,
  value: MobileChord | keyof typeof SHORTCUT_LABELS,
  label: MessageKey,
  options: Pick<MobileAgentKeyboardEntry, "condition"> = {},
): MobileAgentKeyboardEntry {
  const chord = Object.hasOwn(MOBILE_CHORDS, value) ? MOBILE_CHORDS[value as MobileChord] : undefined;
  const physicalLabel = chord?.label ?? SHORTCUT_LABELS[value as keyof typeof SHORTCUT_LABELS];
  return {
    key: {
      id: `agent-${agent}-${value}`,
      kind: chord ? "chord" : "shortcut",
      value,
      label: physicalLabel,
      ariaLabel: physicalLabel,
      width: "md",
      hidden: false,
      repeat: false,
      autoEnter: false,
      custom: false,
    },
    label,
    ...options,
  };
}

function group(id: MobileAgentKeyboardGroup["id"], keys: readonly MobileAgentKeyboardEntry[]): MobileAgentKeyboardGroup {
  return { id, label: `agentKeyboard.group.${id}`, keys };
}

function editingKeys(agent: MobileKeyboardAgent): MobileAgentKeyboardEntry[] {
  return [
    ...(agent === "claude" ? [] : [
      entry(agent, "ctrl-b", agent === "opencode" ? "agentKeyboard.action.characterLeftOrBackground" : "agentKeyboard.action.characterLeft"),
      entry(agent, "ctrl-f", agent === "opencode" ? "agentKeyboard.action.characterRightOrPin" : "agentKeyboard.action.characterRight"),
    ]),
    entry(agent, "alt-b", "agentKeyboard.action.wordLeft"),
    entry(agent, "alt-f", "agentKeyboard.action.wordRight"),
    ...(agent === "claude" || agent === "codex" ? [
      entry(agent, "ctrl-p", "agentKeyboard.action.previousHistory"),
      entry(agent, "ctrl-n", "agentKeyboard.action.nextHistory"),
    ] : []),
    entry(agent, "ctrl-u", agent === "opencode" ? "agentKeyboard.action.deleteToStartOrUndoQueue" : "agentKeyboard.action.deleteToStart"),
    entry(agent, "ctrl-k", "agentKeyboard.action.deleteToEnd"),
    entry(agent, "ctrl-w", "agentKeyboard.action.deleteWord"),
    entry(agent, "alt-d", "agentKeyboard.action.deleteNextWord"),
    ...(agent === "opencode" ? [] : [
      entry(agent, "ctrl-y", "agentKeyboard.action.yank"),
      entry(agent, "alt-y", "agentKeyboard.action.yankNext", { condition: "agentKeyboard.condition.afterYank" }),
    ]),
  ];
}

function controlKeys(agent: MobileKeyboardAgent): MobileAgentKeyboardEntry[] {
  return [
    entry(agent, "ctrl-c", "agentKeyboard.action.interruptCancel"),
    ...(agent === "claude" ? [entry(agent, "ctrl-l", "agentKeyboard.action.redrawScreen")] : []),
    entry(agent, "ctrl-d", "agentKeyboard.action.endInput", { condition: "agentKeyboard.condition.emptyInput" }),
  ];
}

const PROFILES: Record<MobileKeyboardAgent, readonly MobileAgentKeyboardGroup[]> = {
  codex: [
    group("send", [
      entry("codex", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.codexSend" }),
      entry("codex", "ctrl-j", "agentKeyboard.action.newline"),
      entry("codex", "shift-enter", "agentKeyboard.action.newline"),
      entry("codex", "alt-enter", "agentKeyboard.action.newline"),
      entry("codex", "tab", "agentKeyboard.action.queue", { condition: "agentKeyboard.condition.codexQueue" }),
      entry("codex", "alt-up", "agentKeyboard.action.retrieveEdit", { condition: "agentKeyboard.condition.codexRetrieve" }),
      entry("codex", "shift-left", "agentKeyboard.action.retrieveEdit", { condition: "agentKeyboard.condition.codexShiftLeft" }),
    ]),
    group("control", [
      entry("codex", "escape", "agentKeyboard.action.interruptCancel"),
      entry("codex", "alt-down", "agentKeyboard.action.towardInput", { condition: "agentKeyboard.condition.codexNavigate" }),
      entry("codex", "shift-right", "agentKeyboard.action.towardInput", { condition: "agentKeyboard.condition.codexShiftRight" }),
      entry("codex", "ctrl-bracket-right", "agentKeyboard.action.skipQuestion", { condition: "agentKeyboard.condition.codexSkip" }),
      entry("codex", "alt-comma", "agentKeyboard.action.lowerEffort"),
      entry("codex", "shift-down", "agentKeyboard.action.lowerEffort", { condition: "agentKeyboard.condition.codexShiftVertical" }),
      entry("codex", "alt-period", "agentKeyboard.action.raiseEffort"),
      entry("codex", "shift-up", "agentKeyboard.action.raiseEffort", { condition: "agentKeyboard.condition.codexShiftVertical" }),
      entry("codex", "escape-escape", "agentKeyboard.action.editPrevious", { condition: "agentKeyboard.condition.emptyInput" }),
      entry("codex", "f8", "agentKeyboard.action.voice"),
      entry("codex", "ctrl-x", "agentKeyboard.action.voiceMute", { condition: "agentKeyboard.condition.voice" }),
      ...controlKeys("codex"),
    ]),
    group("mode", [
      entry("codex", "alt-left", "agentKeyboard.action.previousAgent"),
      entry("codex", "alt-right", "agentKeyboard.action.nextAgent"),
      entry("codex", "shift-tab", "agentKeyboard.action.collaboration"),
      entry("codex", "alt-r", "agentKeyboard.action.rawOutput"),
      entry("codex", "ctrl-slash", "agentKeyboard.action.sideConversation"),
    ]),
    group("edit", [
      entry("codex", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("codex", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("codex", "ctrl-r", "agentKeyboard.action.historySearch"),
      entry("codex", "ctrl-t", "agentKeyboard.action.transcript"),
      entry("codex", "ctrl-g", "agentKeyboard.action.externalEditor"),
      entry("codex", "ctrl-o", "agentKeyboard.action.copyMessage"),
      entry("codex", "ctrl-l", "agentKeyboard.action.clearTerminal"),
      entry("codex", "ctrl-s", "agentKeyboard.action.historySearchNext"),
      entry("codex", "pageUp", "agentKeyboard.action.pageUp", { condition: "agentKeyboard.condition.pager" }),
      entry("codex", "pageDown", "agentKeyboard.action.pageDown", { condition: "agentKeyboard.condition.pager" }),
      entry("codex", "home", "agentKeyboard.action.lineStart"),
      entry("codex", "end", "agentKeyboard.action.lineEnd"),
      entry("codex", "ctrl-home", "agentKeyboard.action.scrollTop", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("codex", "ctrl-end", "agentKeyboard.action.scrollBottom", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("codex", "shift-home", "agentKeyboard.action.selectLineStart", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("codex", "shift-end", "agentKeyboard.action.selectLineEnd", { condition: "agentKeyboard.condition.fullscreen" }),
      ...editingKeys("codex"),
    ]),
  ],
  claude: [
    group("send", [
      entry("claude", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.claudeSend" }),
      entry("claude", "ctrl-j", "agentKeyboard.action.newline"),
      entry("claude", "shift-enter", "agentKeyboard.action.newline"),
      entry("claude", "ctrl-x-ctrl-s", "agentKeyboard.action.sendQueuedNow", { condition: "agentKeyboard.condition.queuedMessages" }),
      entry("claude", "ctrl-enter", "agentKeyboard.action.sendQueuedNow", { condition: "agentKeyboard.condition.extendedKeys" }),
      entry("claude", "ctrl-x-enter", "agentKeyboard.action.queue", { condition: "agentKeyboard.condition.claudeQueue" }),
      entry("claude", "up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.claudeRetrieve" }),
      entry("claude", "tab", "agentKeyboard.action.autocomplete"),
    ]),
    group("control", [
      entry("claude", "escape", "agentKeyboard.action.interruptCancel"),
      entry("claude", "escape-escape", "agentKeyboard.action.editPrevious"),
      ...controlKeys("claude"),
      entry("claude", "ctrl-g", "agentKeyboard.action.externalEditor"),
      entry("claude", "ctrl-x-ctrl-e", "agentKeyboard.action.externalEditor"),
      entry("claude", "ctrl-b", "agentKeyboard.action.backgroundTask"),
      entry("claude", "ctrl-x-ctrl-b", "agentKeyboard.action.backgroundTask"),
      entry("claude", "ctrl-x-ctrl-k", "agentKeyboard.action.stopBackground", { condition: "agentKeyboard.condition.confirmTwice" }),
      entry("claude", "ctrl-z", "agentKeyboard.action.suspend", { condition: "agentKeyboard.condition.unix" }),
    ]),
    group("mode", [
      entry("claude", "shift-tab", "agentKeyboard.action.permissions"),
      entry("claude", "alt-p", "agentKeyboard.action.model"),
      entry("claude", "alt-t", "agentKeyboard.action.extendedThinking"),
      entry("claude", "alt-o", "agentKeyboard.action.fastMode"),
      entry("claude", "ctrl-t", "agentKeyboard.action.taskList"),
      entry("claude", "ctrl-s", "agentKeyboard.action.stashPrompt"),
    ]),
    group("edit", [
      entry("claude", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("claude", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("claude", "ctrl-r", "agentKeyboard.action.historySearch"),
      entry("claude", "ctrl-o", "agentKeyboard.action.details"),
      entry("claude", "ctrl-minus", "agentKeyboard.action.undoEdit"),
      entry("claude", "down", "agentKeyboard.action.nextHistory"),
      entry("claude", "pageUp", "agentKeyboard.action.pageUp", { condition: "agentKeyboard.condition.dialog" }),
      entry("claude", "pageDown", "agentKeyboard.action.pageDown", { condition: "agentKeyboard.condition.dialog" }),
      entry("claude", "shift-left", "agentKeyboard.action.selectOrHistory", { condition: "agentKeyboard.condition.claudeSelection" }),
      entry("claude", "shift-right", "agentKeyboard.action.selectOrHistory", { condition: "agentKeyboard.condition.claudeSelection" }),
      entry("claude", "shift-up", "agentKeyboard.action.extendSelection", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("claude", "shift-down", "agentKeyboard.action.extendSelection", { condition: "agentKeyboard.condition.fullscreen" }),
      ...editingKeys("claude"),
    ]),
  ],
  pi: [
    group("send", [
      entry("pi", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.piSend" }),
      entry("pi", "ctrl-j", "agentKeyboard.action.newline"),
      entry("pi", "shift-enter", "agentKeyboard.action.newline"),
      entry("pi", "alt-enter", "agentKeyboard.action.followUp", { condition: "agentKeyboard.condition.piFollowUp" }),
      entry("pi", "alt-up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.queuedMessages" }),
    ]),
    group("control", [
      entry("pi", "escape", "agentKeyboard.action.interruptCancel"),
      ...controlKeys("pi"),
      entry("pi", "ctrl-g", "agentKeyboard.action.externalEditor"),
      entry("pi", "ctrl-z", "agentKeyboard.action.suspend", { condition: "agentKeyboard.condition.unix" }),
    ]),
    group("mode", [
      entry("pi", "shift-tab", "agentKeyboard.action.thinking"),
      entry("pi", "ctrl-l", "agentKeyboard.action.model"),
      entry("pi", "ctrl-p", "agentKeyboard.action.nextModel"),
      entry("pi", "ctrl-shift-p", "agentKeyboard.action.previousModel", { condition: "agentKeyboard.condition.extendedKeys" }),
      entry("pi", "ctrl-t", "agentKeyboard.action.toggleThinking"),
      entry("pi", "ctrl-x", "agentKeyboard.action.copyMessage"),
    ]),
    group("edit", [
      entry("pi", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("pi", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("pi", "ctrl-o", "agentKeyboard.action.toolOutput"),
      entry("pi", "ctrl-minus", "agentKeyboard.action.undoEdit"),
      entry("pi", "up", "agentKeyboard.action.previousHistory"),
      entry("pi", "down", "agentKeyboard.action.nextHistory"),
      entry("pi", "pageUp", "agentKeyboard.action.pageUp"),
      entry("pi", "pageDown", "agentKeyboard.action.pageDown"),
      entry("pi", "ctrl-shift-up", "agentKeyboard.action.previousPrompt", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("pi", "ctrl-shift-down", "agentKeyboard.action.nextPrompt", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("pi", "ctrl-home", "agentKeyboard.action.scrollTop", { condition: "agentKeyboard.condition.fullscreen" }),
      entry("pi", "ctrl-end", "agentKeyboard.action.scrollBottom", { condition: "agentKeyboard.condition.fullscreen" }),
      ...editingKeys("pi"),
    ]),
  ],
  opencode: [
    group("send", [
      entry("opencode", "enter", "agentKeyboard.action.send"),
      entry("opencode", "ctrl-j", "agentKeyboard.action.newline"),
      entry("opencode", "shift-enter", "agentKeyboard.action.newline"),
      entry("opencode", "alt-enter", "agentKeyboard.action.newline"),
      entry("opencode", "ctrl-x-enter", "agentKeyboard.action.queue"),
    ]),
    group("control", [
      entry("opencode", "escape", "agentKeyboard.action.interruptCancel"),
      ...controlKeys("opencode"),
      entry("opencode", "ctrl-z", "agentKeyboard.action.suspend", { condition: "agentKeyboard.condition.unix" }),
    ]),
    group("mode", [
      entry("opencode", "ctrl-x-a", "agentKeyboard.action.agentList"),
      entry("opencode", "ctrl-x-m", "agentKeyboard.action.modelList"),
      entry("opencode", "shift-tab", "agentKeyboard.action.nextAgent"),
      entry("opencode", "f2", "agentKeyboard.action.nextModel"),
      entry("opencode", "shift-f2", "agentKeyboard.action.previousModel"),
      entry("opencode", "ctrl-p", "agentKeyboard.action.commandList"),
      entry("opencode", "ctrl-a", "agentKeyboard.action.providerList"),
      entry("opencode", "ctrl-x-n", "agentKeyboard.action.newSession"),
      entry("opencode", "ctrl-x-l", "agentKeyboard.action.sessionList"),
      entry("opencode", "ctrl-o", "agentKeyboard.action.recentSessions"),
      entry("opencode", "ctrl-x-g", "agentKeyboard.action.sessionTimeline"),
      entry("opencode", "ctrl-x-i", "agentKeyboard.action.viewImages"),
      entry("opencode", "ctrl-x-w", "agentKeyboard.action.closeTab"),
      entry("opencode", "ctrl-shift-t", "agentKeyboard.action.reopenTab", { condition: "agentKeyboard.condition.extendedKeys" }),
      ...(["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"] as const)
        .map((number) => entry("opencode", `ctrl-x-${number}`, "agentKeyboard.action.selectTab")),
      entry("opencode", "ctrl-x-c", "agentKeyboard.action.compact"),
      entry("opencode", "ctrl-x-e", "agentKeyboard.action.externalEditor"),
      entry("opencode", "ctrl-x-t", "agentKeyboard.action.terminalToggle"),
      entry("opencode", "ctrl-x-down", "agentKeyboard.action.terminalSelect"),
      entry("opencode", "ctrl-x-up", "agentKeyboard.action.terminalClose"),
      entry("opencode", "ctrl-x-left", "agentKeyboard.action.paneLeft"),
      entry("opencode", "ctrl-x-right", "agentKeyboard.action.paneRight"),
      entry("opencode", "ctrl-x-b", "agentKeyboard.action.sidebar"),
      entry("opencode", "ctrl-x-s", "agentKeyboard.action.status"),
      entry("opencode", "ctrl-x-x", "agentKeyboard.action.exportSession"),
      entry("opencode", "ctrl-x-y", "agentKeyboard.action.copyMessage"),
      entry("opencode", "ctrl-x-u", "agentKeyboard.action.undoMessage"),
      entry("opencode", "ctrl-x-r", "agentKeyboard.action.redoMessage"),
      entry("opencode", "ctrl-r", "agentKeyboard.action.renameSession"),
      entry("opencode", "ctrl-x-q", "agentKeyboard.action.queuedPrompts", { condition: "agentKeyboard.condition.opencodeConflict" }),
      entry("opencode", "ctrl-t", "agentKeyboard.action.variant"),
      entry("opencode", "ctrl-g", "agentKeyboard.action.scrollTop"),
      entry("opencode", "alt-up", "agentKeyboard.action.previousTab", { condition: "agentKeyboard.condition.opencodeTabs" }),
      entry("opencode", "alt-down", "agentKeyboard.action.nextTab", { condition: "agentKeyboard.condition.opencodeTabs" }),
      entry("opencode", "ctrl-shift-tab", "agentKeyboard.action.previousTab", { condition: "agentKeyboard.condition.extendedKeys" }),
    ]),
    group("edit", [
      entry("opencode", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("opencode", "up", "agentKeyboard.action.previousHistory"),
      entry("opencode", "down", "agentKeyboard.action.nextHistory"),
      entry("opencode", "pageUp", "agentKeyboard.action.pageUp"),
      entry("opencode", "pageDown", "agentKeyboard.action.pageDown"),
      entry("opencode", "home", "agentKeyboard.action.scrollTop"),
      entry("opencode", "end", "agentKeyboard.action.scrollBottom"),
      entry("opencode", "alt-home", "agentKeyboard.action.scrollTop"),
      entry("opencode", "alt-end", "agentKeyboard.action.scrollBottom"),
      entry("opencode", "shift-left", "agentKeyboard.action.extendSelection"),
      entry("opencode", "shift-right", "agentKeyboard.action.extendSelection"),
      entry("opencode", "shift-up", "agentKeyboard.action.extendSelection"),
      entry("opencode", "shift-down", "agentKeyboard.action.extendSelection"),
      entry("opencode", "ctrl-alt-b", "agentKeyboard.action.pageUp"),
      entry("opencode", "ctrl-alt-f", "agentKeyboard.action.pageDown"),
      entry("opencode", "ctrl-alt-y", "agentKeyboard.action.lineUp"),
      entry("opencode", "ctrl-alt-e", "agentKeyboard.action.lineDown"),
      entry("opencode", "ctrl-alt-u", "agentKeyboard.action.halfPageUp"),
      entry("opencode", "ctrl-alt-d", "agentKeyboard.action.halfPageDown"),
      entry("opencode", "ctrl-alt-g", "agentKeyboard.action.scrollBottom"),
      ...editingKeys("opencode"),
    ]),
  ],
};

export function mobileAgentKeyboardGroups(agent: MobileKeyboardAgent): readonly MobileAgentKeyboardGroup[] {
  return PROFILES[agent];
}

export function mobileAgentName(agent: MobileKeyboardAgent): string {
  return { claude: "Claude Code", codex: "Codex CLI", opencode: "OpenCode", pi: "Pi" }[agent];
}
