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

const SHORTCUT_LABELS = { enter: "Enter", escape: "Esc", tab: "Tab", up: "Up", down: "Down" } as const;

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
      entry(agent, "ctrl-b", "agentKeyboard.action.characterLeft"),
      entry(agent, "ctrl-f", "agentKeyboard.action.characterRight"),
    ]),
    entry(agent, "alt-b", "agentKeyboard.action.wordLeft"),
    entry(agent, "alt-f", "agentKeyboard.action.wordRight"),
    ...(agent === "claude" || agent === "codex" ? [
      entry(agent, "ctrl-p", "agentKeyboard.action.previousHistory"),
      entry(agent, "ctrl-n", "agentKeyboard.action.nextHistory"),
    ] : []),
    entry(agent, "ctrl-u", "agentKeyboard.action.deleteToStart"),
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
      entry("codex", "tab", "agentKeyboard.action.queue", { condition: "agentKeyboard.condition.codexQueue" }),
      entry("codex", "alt-up", "agentKeyboard.action.retrieveEdit", { condition: "agentKeyboard.condition.codexRetrieve" }),
    ]),
    group("control", [
      entry("codex", "escape", "agentKeyboard.action.interruptCancel"),
      entry("codex", "alt-down", "agentKeyboard.action.towardInput", { condition: "agentKeyboard.condition.codexNavigate" }),
      entry("codex", "ctrl-bracket-right", "agentKeyboard.action.skipQuestion", { condition: "agentKeyboard.condition.codexSkip" }),
      ...controlKeys("codex"),
    ]),
    group("mode", [
      entry("codex", "alt-left", "agentKeyboard.action.previousAgent"),
      entry("codex", "alt-right", "agentKeyboard.action.nextAgent"),
      entry("codex", "shift-tab", "agentKeyboard.action.collaboration"),
    ]),
    group("edit", [
      entry("codex", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("codex", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("codex", "ctrl-r", "agentKeyboard.action.historySearch"),
      entry("codex", "ctrl-t", "agentKeyboard.action.transcript"),
      ...editingKeys("codex"),
    ]),
  ],
  claude: [
    group("send", [
      entry("claude", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.claudeSend" }),
      entry("claude", "ctrl-j", "agentKeyboard.action.newline"),
      entry("claude", "ctrl-x-ctrl-s", "agentKeyboard.action.sendQueuedNow", { condition: "agentKeyboard.condition.queuedMessages" }),
      entry("claude", "up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.claudeRetrieve" }),
      entry("claude", "tab", "agentKeyboard.action.autocomplete"),
    ]),
    group("control", [
      entry("claude", "escape", "agentKeyboard.action.interruptCancel"),
      ...controlKeys("claude"),
      entry("claude", "ctrl-g", "agentKeyboard.action.externalEditor"),
      entry("claude", "ctrl-x-ctrl-e", "agentKeyboard.action.externalEditor"),
      entry("claude", "ctrl-b", "agentKeyboard.action.backgroundTask"),
      entry("claude", "ctrl-x-ctrl-k", "agentKeyboard.action.stopBackground", { condition: "agentKeyboard.condition.confirmTwice" }),
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
      ...editingKeys("claude"),
    ]),
  ],
  pi: [
    group("send", [
      entry("pi", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.piSend" }),
      entry("pi", "ctrl-j", "agentKeyboard.action.newline"),
      entry("pi", "alt-enter", "agentKeyboard.action.followUp", { condition: "agentKeyboard.condition.piFollowUp" }),
      entry("pi", "alt-up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.queuedMessages" }),
    ]),
    group("control", [
      entry("pi", "escape", "agentKeyboard.action.interruptCancel"),
      ...controlKeys("pi"),
      entry("pi", "ctrl-g", "agentKeyboard.action.externalEditor"),
    ]),
    group("mode", [
      entry("pi", "shift-tab", "agentKeyboard.action.thinking"),
      entry("pi", "ctrl-l", "agentKeyboard.action.model"),
      entry("pi", "ctrl-p", "agentKeyboard.action.nextModel"),
      entry("pi", "ctrl-t", "agentKeyboard.action.toggleThinking"),
    ]),
    group("edit", [
      entry("pi", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("pi", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("pi", "ctrl-o", "agentKeyboard.action.toolOutput"),
      ...editingKeys("pi"),
    ]),
  ],
  opencode: [
    group("send", [
      entry("opencode", "enter", "agentKeyboard.action.send"),
      entry("opencode", "ctrl-j", "agentKeyboard.action.newline"),
      entry("opencode", "ctrl-x-enter", "agentKeyboard.action.queue"),
    ]),
    group("control", [entry("opencode", "escape", "agentKeyboard.action.interruptCancel"), ...controlKeys("opencode")]),
    group("mode", [
      entry("opencode", "ctrl-x-a", "agentKeyboard.action.agentList"),
      entry("opencode", "ctrl-x-m", "agentKeyboard.action.modelList"),
      entry("opencode", "ctrl-p", "agentKeyboard.action.commandList"),
      entry("opencode", "ctrl-a", "agentKeyboard.action.providerList"),
      entry("opencode", "ctrl-x-n", "agentKeyboard.action.newSession"),
      entry("opencode", "ctrl-x-l", "agentKeyboard.action.sessionList"),
      entry("opencode", "ctrl-x-g", "agentKeyboard.action.sessionTimeline"),
      entry("opencode", "ctrl-x-c", "agentKeyboard.action.compact"),
      entry("opencode", "ctrl-x-e", "agentKeyboard.action.externalEditor"),
      entry("opencode", "ctrl-x-t", "agentKeyboard.action.themeList"),
      entry("opencode", "ctrl-x-b", "agentKeyboard.action.sidebar"),
      entry("opencode", "ctrl-x-s", "agentKeyboard.action.status"),
      entry("opencode", "ctrl-x-x", "agentKeyboard.action.exportSession"),
      entry("opencode", "ctrl-x-y", "agentKeyboard.action.copyMessage"),
      entry("opencode", "ctrl-x-u", "agentKeyboard.action.undoMessage"),
      entry("opencode", "ctrl-x-r", "agentKeyboard.action.redoMessage"),
      entry("opencode", "ctrl-r", "agentKeyboard.action.renameSession"),
      entry("opencode", "ctrl-x-q", "agentKeyboard.action.exit"),
      entry("opencode", "ctrl-t", "agentKeyboard.action.variant"),
    ]),
    group("edit", [
      entry("opencode", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("opencode", "up", "agentKeyboard.action.previousHistory"),
      entry("opencode", "down", "agentKeyboard.action.nextHistory"),
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
