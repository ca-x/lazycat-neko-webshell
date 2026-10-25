import type {
  MobileKeyboardKey,
  MobileKeyboardKeyKind,
  MobileKeyboardKeyWidth,
  MobileKeyboardLayout,
  MobileKeyboardPage,
  MobileKeyboardPageId,
  MobileKeyboardPresetId,
} from "./keyboard-layout-types.ts";
import { newId } from "../utils.ts";
import { MOBILE_CHORDS, MOBILE_KEYBOARD_CHORDS } from "./key-chords.ts";

export const MAX_MOBILE_KEY_TEXT = 256;
export const MAX_MOBILE_KEY_LABEL = 24;
export const MAX_MOBILE_KEYS_PER_PAGE = 32;

export const MOBILE_KEYBOARD_PAGE_IDS: MobileKeyboardPageId[] = ["main", "ops", "nav", "fn", "sym"];
export const MOBILE_KEYBOARD_ACTIONS = [
  "previous-tab",
  "next-tab",
  "new-tab",
  "close-tab",
  "previous-pane",
  "next-pane",
  "split-right",
  "split-down",
  "copy-selection",
  "paste-clipboard",
  "secret-file",
  "compose-text",
  "command-palette",
  "font-larger",
  "font-smaller",
  "pane-menu",
  "swap-pane",
  "maximize-pane",
  "workspace-overview",
] as const;

const ACTIONS = new Set<string>(MOBILE_KEYBOARD_ACTIONS);
const CHORDS = new Set<string>(MOBILE_KEYBOARD_CHORDS);
const SAFE_REPEAT = new Set([
  "left", "right", "up", "down", "home", "end", "pageUp", "pageDown", "delete", "backspace", "enter",
]);
const NAMED_SHORTCUTS = new Set([
  "ctrl", "alt", "shift", "left", "right", "up", "down", "tab", "enter", "escape", "home", "end",
  "pageUp", "pageDown", "insert", "delete", "backspace", "paste",
  ...Array.from({ length: 12 }, (_, index) => `f${index + 1}`),
]);

type KeySeed = Partial<MobileKeyboardKey> & Pick<MobileKeyboardKey, "id" | "kind" | "value" | "label">;

function key(seed: KeySeed): MobileKeyboardKey {
  return {
    ariaLabel: seed.ariaLabel ?? seed.label,
    width: seed.width ?? "md",
    hidden: seed.hidden ?? false,
    repeat: seed.repeat ?? false,
    autoEnter: seed.autoEnter ?? false,
    custom: seed.custom ?? false,
    icon: seed.icon,
    id: seed.id,
    kind: seed.kind,
    value: seed.value,
    label: seed.label,
  };
}

function shortcut(id: string, value: string, label: string, options: Partial<MobileKeyboardKey> = {}) {
  return key({ id, kind: "shortcut", value, label, ...options });
}

function action(id: string, value: string, label: string, icon?: string, ariaLabel = label) {
  return key({ id, kind: "action", value, label, ariaLabel, icon });
}

function chord(id: string, value: string, label: string) {
  return key({ id, kind: "chord", value, label });
}

const symbols = ["$", "/", "-", "~", "|", "\\", "`", "[", "]", "{", "}", "(", ")", "&", ";", ":", "*", "?", ".", ",", "=", "'"];

function defaultPages(): MobileKeyboardPage[] {
  return [
    { id: "main", keys: [
      shortcut("main-escape", "escape", "Esc"),
      shortcut("main-tab", "tab", "Tab"), chord("main-shift-tab", "shift-tab", "Shift+Tab"), shortcut("main-enter", "enter", "Return"),
      chord("main-ctrl-c", "ctrl-c", "Ctrl+C"), chord("main-ctrl-r", "ctrl-r", "Ctrl+R"),
      action("main-copy", "copy-selection", "Copy"), shortcut("main-paste", "paste", "", { icon: "clipboard-paste", ariaLabel: "Paste" }),
      shortcut("main-ctrl", "ctrl", "Ctrl"), shortcut("main-alt", "alt", "Alt"), shortcut("main-shift", "shift", "Shift"),
      chord("main-ctrl-a", "ctrl-a", "Ctrl+A"), chord("main-ctrl-e", "ctrl-e", "Ctrl+E"),
      chord("main-ctrl-u", "ctrl-u", "Ctrl+U"), chord("main-ctrl-w", "ctrl-w", "Ctrl+W"),
      chord("main-ctrl-k", "ctrl-k", "Ctrl+K"), chord("main-ctrl-l", "ctrl-l", "Ctrl+L"),
      chord("main-alt-b", "alt-b", "Alt+B"), chord("main-alt-f", "alt-f", "Alt+F"),
      chord("main-alt-period", "alt-period", "Alt+."),
    ] },
    { id: "ops", keys: [
      action("ops-overview", "workspace-overview", "", "layout-grid", "Tabs and panes"),
      action("ops-prev-tab", "previous-tab", "Tab", "chevron-left", "Previous terminal tab"), action("ops-next-tab", "next-tab", "Tab", "chevron-right", "Next terminal tab"),
      action("ops-new-tab", "new-tab", "", "square-plus", "New terminal tab"), action("ops-close-tab", "close-tab", "", "square-x", "Close tab"),
      action("ops-prev-pane", "previous-pane", "Pane", "chevron-left", "Previous pane"), action("ops-next-pane", "next-pane", "Pane", "chevron-right", "Next pane"),
      action("ops-split-right", "split-right", "", "panel-right", "Split right"), action("ops-split-down", "split-down", "", "panel-bottom", "Split down"),
      action("ops-copy", "copy-selection", "", "copy", "Copy selection"), action("ops-paste", "paste-clipboard", "", "clipboard-paste", "Paste"),
      action("ops-font-up", "font-larger", "A+"), action("ops-font-down", "font-smaller", "A-"),
      action("ops-menu", "pane-menu", "Menu"),
    ] },
    { id: "nav", keys: [
      shortcut("nav-left", "left", "", { icon: "arrow-left", ariaLabel: "Left", repeat: true }),
      shortcut("nav-down", "down", "", { icon: "arrow-down", ariaLabel: "Down", repeat: true }),
      shortcut("nav-up", "up", "", { icon: "arrow-up", ariaLabel: "Up", repeat: true }),
      shortcut("nav-right", "right", "", { icon: "arrow-right", ariaLabel: "Right", repeat: true }),
      shortcut("nav-home", "home", "Home", { repeat: true }), shortcut("nav-end", "end", "End", { repeat: true }),
      shortcut("nav-page-up", "pageUp", "PgUp", { repeat: true }), shortcut("nav-page-down", "pageDown", "PgDn", { repeat: true }),
      shortcut("nav-insert", "insert", "Ins"), shortcut("nav-delete", "delete", "Del", { repeat: true }),
      shortcut("nav-backspace", "backspace", "Bksp", { repeat: true }),
    ] },
    { id: "fn", keys: Array.from({ length: 12 }, (_, index) => shortcut(`fn-${index + 1}`, `f${index + 1}`, `F${index + 1}`)) },
    { id: "sym", keys: symbols.map((value, index) => shortcut(`sym-${index}`, value, value)) },
  ];
}

function operationsPages(): MobileKeyboardPage[] {
  const pages = defaultPages();
  const main = pages.find((page) => page.id === "main");
  const ops = pages.find((page) => page.id === "ops");
  if (main) {
    const wanted = new Set(["main-ctrl", "main-alt", "main-shift", "main-left", "main-down", "main-up", "main-right", "main-tab", "main-enter", "main-escape"]);
    main.keys.forEach((item) => { item.hidden = !wanted.has(item.id); });
  }
  ops?.keys.unshift(
    action("ops-maximize", "maximize-pane", "Maximize", "maximize-2"),
  );
  return pages;
}

function editorPages(): MobileKeyboardPage[] {
  const pages = defaultPages();
  const main = pages.find((page) => page.id === "main");
  if (main) {
    const order = ["main-ctrl", "main-alt", "main-shift", "main-escape", "main-tab", "main-enter", "main-left", "main-down", "main-up", "main-right", "main-ctrl-c", "main-shift-tab"];
    main.keys.sort((left, right) => {
      const leftIndex = order.indexOf(left.id);
      const rightIndex = order.indexOf(right.id);
      return (leftIndex < 0 ? 999 : leftIndex) - (rightIndex < 0 ? 999 : rightIndex);
    });
    main.keys.forEach((item) => { item.hidden = !order.includes(item.id); });
  }
  return pages;
}

function vimPages(): MobileKeyboardPage[] {
  const pages = defaultPages();
  pages[0] = { id: "main", keys: [
    shortcut("vim-escape", "escape", "Esc"), shortcut("vim-colon", ":", ":"),
    shortcut("vim-search", "/", "/"), shortcut("vim-reverse-search", "?", "?"), shortcut("vim-enter", "enter", "Return"),
    ...[..."hjkl"].map((value) => shortcut(`vim-${value}`, value, value)),
    shortcut("vim-undo", "u", "u"), chord("vim-redo", "ctrl-r", "Ctrl+R"),
    ...[..."iavwbe0$"].map((value) => shortcut(`vim-key-${value.charCodeAt(0)}`, value, value)),
    chord("vim-shift-tab", "shift-tab", "Shift+Tab"),
  ] };
  return pages;
}

function emacsPages(): MobileKeyboardPage[] {
  const pages = defaultPages();
  const chords = ["ctrl-g", "ctrl-x", "alt-x", "ctrl-x-ctrl-s", "ctrl-x-ctrl-f", "ctrl-a", "ctrl-e", "alt-b", "alt-f", "ctrl-p", "ctrl-n", "ctrl-b", "ctrl-f", "ctrl-space", "ctrl-w", "alt-w", "ctrl-y", "ctrl-k", "ctrl-s", "ctrl-r"] as const;
  pages[0] = { id: "main", keys: chords.map((value) => chord(`emacs-${value}`, value, MOBILE_CHORDS[value].label)) };
  return pages;
}

export function normalizeMobileKeyboardPreset(value: unknown): MobileKeyboardPresetId {
  return value === "operations" || value === "editor" || value === "vim" || value === "emacs" || value === "custom" ? value : "default";
}

export function mobileKeyboardPresetLayout(preset: Exclude<MobileKeyboardPresetId, "custom">): MobileKeyboardLayout {
  const pages = preset === "operations" ? operationsPages() : preset === "editor" ? editorPages() : preset === "vim" ? vimPages() : preset === "emacs" ? emacsPages() : defaultPages();
  return { pages };
}

export function resolveMobileKeyboardLayout(
  preset: MobileKeyboardPresetId,
  custom: MobileKeyboardLayout,
): MobileKeyboardLayout {
  return preset === "custom" ? normalizeMobileKeyboardLayout(custom) : mobileKeyboardPresetLayout(preset);
}

export function normalizeMobileKeyboardLayout(value: unknown): MobileKeyboardLayout {
  const rawPages = value && typeof value === "object" && Array.isArray((value as Partial<MobileKeyboardLayout>).pages)
    ? (value as Partial<MobileKeyboardLayout>).pages ?? []
    : [];
  const fallbackPages = defaultPages();
  const pages = MOBILE_KEYBOARD_PAGE_IDS.map((id) => {
    const raw = rawPages.find((page) => page && typeof page === "object" && (page as Partial<MobileKeyboardPage>).id === id);
    const keys = raw && Array.isArray((raw as Partial<MobileKeyboardPage>).keys)
      ? normalizeKeys((raw as Partial<MobileKeyboardPage>).keys ?? [])
      : [];
    const fallback = fallbackPages.find((page) => page.id === id)?.keys ?? [];
    return { id, keys: keys.length ? keys : fallback };
  });
  return { pages };
}

function normalizeKeys(value: unknown[]): MobileKeyboardKey[] {
  const keys: MobileKeyboardKey[] = [];
  const ids = new Set<string>();
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const raw = item as Partial<MobileKeyboardKey>;
    const kind = normalizeKind(raw.kind);
    const text = typeof raw.value === "string" ? normalizeKeyText(raw.value) : "";
    if (!kind || !validValue(kind, text)) continue;
    const id = normalizeId(raw.id, ids, keys.length);
    const icon = normalizeIcon(raw.icon);
    const normalizedAriaLabel = normalizeLabel(raw.ariaLabel);
    const label = normalizeLabel(raw.label) || (icon && normalizedAriaLabel ? "" : defaultLabel(kind, text));
    const repeat = kind === "shortcut" && SAFE_REPEAT.has(text) && raw.repeat === true;
    keys.push({
      id,
      kind,
      value: text,
      label,
      ariaLabel: normalizedAriaLabel || label || text,
      icon,
      width: normalizeWidth(raw.width),
      hidden: raw.hidden === true,
      repeat,
      autoEnter: kind === "text" && raw.autoEnter === true,
      custom: raw.custom === true || kind === "text",
    });
    ids.add(id);
    if (keys.length >= MAX_MOBILE_KEYS_PER_PAGE) break;
  }
  return keys;
}

function normalizeKind(value: unknown): MobileKeyboardKeyKind | undefined {
  return value === "shortcut" || value === "chord" || value === "action" || value === "text" ? value : undefined;
}

function validValue(kind: MobileKeyboardKeyKind, value: string): boolean {
  if (!value) return false;
  if (kind === "action") return ACTIONS.has(value);
  if (kind === "chord") return CHORDS.has(value);
  if (kind === "shortcut") return NAMED_SHORTCUTS.has(value) || (value.length === 1 && value >= " " && value !== "\x7f");
  return true;
}

function normalizeKeyText(value: string): string {
  return cleanMobileKeyboardText(value).slice(0, MAX_MOBILE_KEY_TEXT);
}

function cleanMobileKeyboardText(value: string): string {
  return value.replace(/\r\n/g, "\n").replace(/[\u0000\u0008\u000b\u000c\u007f]/g, "");
}

export function mobileKeyboardTextError(value: string): "empty" | "tooLong" | undefined {
  const text = cleanMobileKeyboardText(decodeMobileKeyboardText(value));
  return !text.length ? "empty" : text.length > MAX_MOBILE_KEY_TEXT ? "tooLong" : undefined;
}

function normalizeLabel(value: unknown): string {
  return typeof value === "string" ? value.trim().slice(0, MAX_MOBILE_KEY_LABEL) : "";
}

function normalizeIcon(value: unknown): string | undefined {
  return typeof value === "string" && /^[a-z0-9-]{1,32}$/.test(value) ? value : undefined;
}

function normalizeWidth(value: unknown): MobileKeyboardKeyWidth {
  return value === "sm" || value === "lg" ? value : "md";
}

function normalizeId(value: unknown, ids: Set<string>, index: number): string {
  const candidate = typeof value === "string" ? value.replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 64) : "";
  let id = candidate || `key-${index + 1}`;
  let suffix = 2;
  while (ids.has(id)) id = `${candidate || `key-${index + 1}`}-${suffix++}`;
  return id;
}

function defaultLabel(kind: MobileKeyboardKeyKind, value: string): string {
  if (kind === "shortcut" && value.startsWith("f")) return value.toUpperCase();
  return value.slice(0, MAX_MOBILE_KEY_LABEL);
}

export function moveMobileKeyboardKey(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  keyId: string,
  direction: -1 | 1,
): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const page = next.pages.find((item) => item.id === pageId);
  const index = page?.keys.findIndex((item) => item.id === keyId) ?? -1;
  const target = index + direction;
  if (!page || index < 0 || target < 0 || target >= page.keys.length) return next;
  [page.keys[index], page.keys[target]] = [page.keys[target], page.keys[index]];
  return next;
}

export function moveMobileKeyboardKeyToIndex(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  keyId: string,
  targetIndex: number,
): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const page = next.pages.find((item) => item.id === pageId);
  if (!page) return next;
  const sourceIndex = page.keys.findIndex((item) => item.id === keyId);
  if (sourceIndex < 0) return next;
  const clampedIndex = Math.max(0, Math.min(targetIndex, page.keys.length - 1));
  if (sourceIndex === clampedIndex) return next;
  const [item] = page.keys.splice(sourceIndex, 1);
  if (item) page.keys.splice(clampedIndex, 0, item);
  return next;
}

export function updateMobileKeyboardKey(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  keyId: string,
  patch: Partial<Pick<MobileKeyboardKey, "hidden" | "width">>,
): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const item = next.pages.find((page) => page.id === pageId)?.keys.find((candidate) => candidate.id === keyId);
  if (!item) return next;
  if (typeof patch.hidden === "boolean") item.hidden = patch.hidden;
  if (patch.width === "sm" || patch.width === "md" || patch.width === "lg") item.width = patch.width;
  return next;
}

export function addMobileKeyboardTextKey(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  input: { label: string; value: string; width: MobileKeyboardKeyWidth; autoEnter: boolean },
): MobileKeyboardLayout {
  return addMobileKeyboardKey(layout, pageId, { kind: "text", ...input });
}

export function addMobileKeyboardKey(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  input: {
    kind: MobileKeyboardKeyKind;
    label: string;
    value: string;
    width: MobileKeyboardKeyWidth;
    autoEnter: boolean;
  },
): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const page = next.pages.find((item) => item.id === pageId);
  if (!page || page.keys.length >= MAX_MOBILE_KEYS_PER_PAGE) return next;
  const normalized = normalizeKeys([{
    id: `custom-${newId()}`,
    kind: input.kind,
    value: input.kind === "text" ? decodeMobileKeyboardText(input.value) : input.value,
    label: input.label,
    width: input.width,
    autoEnter: input.autoEnter,
    custom: true,
  }])[0];
  if (normalized) page.keys.push(normalized);
  return next;
}

export function decodeMobileKeyboardText(value: string): string {
  return value.replace(/\\(x1b|e|r|n|t|\\)/gi, (match, sequence: string) => {
    if (sequence.toLowerCase() === "x1b" || sequence.toLowerCase() === "e") return "\x1b";
    if (sequence === "r") return "\r";
    if (sequence === "n") return "\n";
    if (sequence === "t") return "\t";
    if (sequence === "\\") return "\\";
    return match;
  });
}

/** Lossless editor representation of supported persisted terminal controls. */
export function encodeMobileKeyboardText(value: string): string {
  return value.replace(/[\\\x1b\r\n\t]/g, (character) => ({
    "\\": "\\\\", "\x1b": "\\e", "\r": "\\r", "\n": "\\n", "\t": "\\t",
  })[character] ?? character);
}

export function editMobileKeyboardCustomKey(
  layout: MobileKeyboardLayout,
  pageId: MobileKeyboardPageId,
  keyId: string,
  input: Pick<MobileKeyboardKey, "kind" | "label" | "value" | "width" | "autoEnter">,
): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const page = next.pages.find((item) => item.id === pageId);
  const index = page?.keys.findIndex((item) => item.id === keyId && item.custom) ?? -1;
  if (!page || index < 0) return next;
  const original = page.keys[index]!;
  const edited = normalizeKeys([{
    ...original, ...input, ariaLabel: input.label, icon: undefined,
    value: input.kind === "text" ? decodeMobileKeyboardText(input.value) : input.value,
    repeat: original.kind === input.kind && original.value === input.value && original.repeat,
  }])[0];
  if (edited) page.keys[index] = edited;
  return next;
}

export function removeMobileKeyboardKey(layout: MobileKeyboardLayout, pageId: MobileKeyboardPageId, keyId: string): MobileKeyboardLayout {
  const next = structuredClone(normalizeMobileKeyboardLayout(layout));
  const page = next.pages.find((item) => item.id === pageId);
  if (page) page.keys = page.keys.filter((item) => item.id !== keyId || !item.custom);
  return next;
}
