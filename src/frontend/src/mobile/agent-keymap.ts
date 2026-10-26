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
    ]),
  ],
  claude: [
    group("send", [
      entry("claude", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.claudeSend" }),
      entry("claude", "ctrl-j", "agentKeyboard.action.newline"),
      entry("claude", "ctrl-x-ctrl-s", "agentKeyboard.action.sendQueuedNow", { condition: "agentKeyboard.condition.queuedMessages" }),
      entry("claude", "up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.claudeRetrieve" }),
    ]),
    group("control", [entry("claude", "escape", "agentKeyboard.action.interruptCancel")]),
    group("mode", [
      entry("claude", "shift-tab", "agentKeyboard.action.permissions"),
      entry("claude", "alt-p", "agentKeyboard.action.model"),
    ]),
    group("edit", [
      entry("claude", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("claude", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("claude", "ctrl-r", "agentKeyboard.action.historySearch"),
      entry("claude", "ctrl-o", "agentKeyboard.action.details"),
    ]),
  ],
  pi: [
    group("send", [
      entry("pi", "enter", "agentKeyboard.action.send", { condition: "agentKeyboard.condition.piSend" }),
      entry("pi", "ctrl-j", "agentKeyboard.action.newline"),
      entry("pi", "alt-enter", "agentKeyboard.action.followUp", { condition: "agentKeyboard.condition.piFollowUp" }),
      entry("pi", "alt-up", "agentKeyboard.action.retrieve", { condition: "agentKeyboard.condition.queuedMessages" }),
    ]),
    group("control", [entry("pi", "escape", "agentKeyboard.action.interruptCancel")]),
    group("mode", [
      entry("pi", "shift-tab", "agentKeyboard.action.thinking"),
      entry("pi", "ctrl-l", "agentKeyboard.action.model"),
    ]),
    group("edit", [
      entry("pi", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("pi", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("pi", "ctrl-o", "agentKeyboard.action.toolOutput"),
    ]),
  ],
  opencode: [
    group("send", [
      entry("opencode", "enter", "agentKeyboard.action.send"),
      entry("opencode", "ctrl-j", "agentKeyboard.action.newline"),
      entry("opencode", "ctrl-x-enter", "agentKeyboard.action.queue"),
    ]),
    group("control", [entry("opencode", "escape", "agentKeyboard.action.interruptCancel")]),
    group("mode", [
      entry("opencode", "ctrl-x-a", "agentKeyboard.action.agentList"),
      entry("opencode", "ctrl-x-m", "agentKeyboard.action.modelList"),
    ]),
    group("edit", [
      entry("opencode", "ctrl-a", "agentKeyboard.action.lineStart"),
      entry("opencode", "ctrl-e", "agentKeyboard.action.lineEnd"),
      entry("opencode", "up", "agentKeyboard.action.previousHistory"),
      entry("opencode", "down", "agentKeyboard.action.nextHistory"),
    ]),
  ],
};

export function mobileAgentKeyboardGroups(agent: MobileKeyboardAgent): readonly MobileAgentKeyboardGroup[] {
  return PROFILES[agent];
}

export function mobileAgentName(agent: MobileKeyboardAgent): string {
  return { claude: "Claude Code", codex: "Codex CLI", opencode: "OpenCode", pi: "Pi" }[agent];
}
