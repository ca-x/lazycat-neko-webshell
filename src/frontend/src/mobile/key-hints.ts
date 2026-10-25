import type { MessageKey } from "../i18n.ts";
import type { MobileKeyboardKey } from "./keyboard-layout-types.ts";

export type MobileKeyTranslate = (key: MessageKey, values?: Record<string, string | number>) => string;

const HINTS: Record<string, MessageKey> = {
  "main-ctrl-a": "mobileKeyboard.lineStart",
  "main-ctrl-e": "mobileKeyboard.lineEnd",
  "main-ctrl-r": "mobileKeyboard.history",
  "main-ctrl-u": "mobileKeyboard.cutBefore",
  "main-ctrl-k": "mobileKeyboard.cutAfter",
  "main-ctrl-w": "mobileKeyboard.deleteWord",
  "main-ctrl-l": "mobileKeyboard.clearScreen",
  "main-alt-b": "mobileKeyboard.wordBack",
  "main-alt-f": "mobileKeyboard.wordForward",
  "main-alt-period": "mobileKeyboard.lastArgument",
};

export function mobileKeyHint(key: MobileKeyboardKey, tr?: MobileKeyTranslate): string {
  return !key.custom && HINTS[key.id] && tr ? tr(HINTS[key.id]!) : "";
}
