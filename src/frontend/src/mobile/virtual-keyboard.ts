import { escapeAttr, escapeHtml } from "../utils.ts";

type VirtualKeyBase = {
  label: string;
  name: string;
  repeat?: boolean;
  size?: "wide" | "space";
};

type VirtualKey = VirtualKeyBase & (
  | { shortcut: string; chord?: never }
  | { chord: "shift-tab"; shortcut?: never }
);

const printable = (value: string, name = value): VirtualKey => ({
  label: value.toUpperCase(),
  name,
  shortcut: value,
});

const shortcut = (value: string, label: string, name = label, repeat = false): VirtualKey => ({
  label,
  name,
  shortcut: value,
  repeat,
});

const SYMBOL_KEYS = [
  ["`", "Backtick"], ["-", "Hyphen"], ["=", "Equals"], ["\\", "Backslash"],
  [";", "Semicolon"], ["'", "Apostrophe"], [",", "Comma"], [".", "Period"], ["/", "Slash"],
] as const;

const TYPING_ROWS: readonly (readonly VirtualKey[])[] = [
  [shortcut("escape", "Esc", "Escape"), ...SYMBOL_KEYS.map(([value, name]) => printable(value, name))],
  [..."1234567890"].map((value) => printable(value)),
  [..."qwertyuiop"].map((value) => printable(value, value.toUpperCase())),
  [..."asdfghjkl"].map((value) => printable(value, value.toUpperCase())),
  [shortcut("shift", "⇧", "Shift"), ...[..."zxcvbnm"].map((value) => printable(value, value.toUpperCase())), shortcut("backspace", "⌫", "Backspace", true)],
  [
    shortcut("ctrl", "Ctrl"), shortcut("alt", "Alt"), shortcut("tab", "Tab"),
    { ...printable(" ", "Space"), label: "Space", size: "space" },
    shortcut("enter", "↵", "Enter", true),
    shortcut("left", "←", "Left arrow", true), shortcut("up", "↑", "Up arrow", true),
    shortcut("down", "↓", "Down arrow", true), shortcut("right", "→", "Right arrow", true),
  ],
];

function renderKey(key: VirtualKey): string {
  const route = key.chord
    ? `data-mobile-chord="${escapeAttr(key.chord)}"`
    : `data-mobile-shortcut="${escapeAttr(key.shortcut)}"`;
  const modifier = key.shortcut === "ctrl" || key.shortcut === "alt" || key.shortcut === "shift"
    ? ` data-mobile-modifier="${key.shortcut}" aria-pressed="false"`
    : "";
  return `<button type="button" class="mobile-virtual-key" ${route} aria-label="${escapeAttr(key.name)}"${modifier}${key.repeat ? ' data-mobile-repeat="true"' : ""}${key.size ? ` data-mobile-virtual-size="${key.size}"` : ""}>${escapeHtml(key.label)}</button>`;
}

export function renderMobileTypingKeyboard(): string {
  return `<div class="mobile-virtual-keyboard-panel" data-mobile-panel="typing" role="group" aria-label="Typing keyboard" data-i18n-aria="label.mobileTypingKeys" hidden>${TYPING_ROWS.map((row, index) => (
    `<div class="mobile-virtual-keyboard-row" data-mobile-virtual-row="${index + 1}">${row.map(renderKey).join("")}</div>`
  )).join("")}</div>`;
}
