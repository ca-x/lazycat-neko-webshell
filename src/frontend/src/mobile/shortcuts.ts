import { encodeMobileShortcutKeyInput, encodeModifiedTextInput, type TerminalShortcutModifiers } from "../keyboard.ts";
import { MOBILE_CHORDS, type MobileChord } from "./key-chords.ts";

export type MobileStickyModifier = "ctrl" | "alt" | "shift";

export type MobileStickyState = Required<Pick<TerminalShortcutModifiers, MobileStickyModifier>>;

export function createMobileStickyState(): MobileStickyState {
  return {
    ctrl: false,
    alt: false,
    shift: false,
  };
}

export function isMobileModifierShortcut(shortcut: string): shortcut is MobileStickyModifier {
  return shortcut === "ctrl" || shortcut === "alt" || shortcut === "shift";
}

export function toggleMobileModifier(state: MobileStickyState, shortcut: MobileStickyModifier) {
  state[shortcut] = !state[shortcut];
}

export function encodeMobileShortcutInput(
  shortcut: string,
  state: MobileStickyState,
): string | undefined {
  return encodeMobileShortcutKeyInput(shortcut, state);
}

export function mobileChordInput(chord: string): string | undefined {
  return Object.hasOwn(MOBILE_CHORDS, chord) ? MOBILE_CHORDS[chord as MobileChord].input : undefined;
}

export function hasMobileStickyModifiers(state: MobileStickyState): boolean {
  return state.ctrl || state.alt || state.shift;
}

export function transformMobileStickyInput(
  state: MobileStickyState,
  text: string,
  source: string,
): string | undefined {
  if (!hasMobileStickyModifiers(state) || source === "pty" || source === "program") return undefined;
  const encoded = encodeModifiedTextInput(text, state);
  if (!encoded) return undefined;
  clearMobileSticky(state);
  return encoded;
}

export function clearMobileSticky(state: MobileStickyState) {
  state.ctrl = false;
  state.alt = false;
  state.shift = false;
}
