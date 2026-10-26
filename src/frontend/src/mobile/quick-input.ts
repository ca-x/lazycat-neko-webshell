import type { MessageKey } from "../i18n.ts";
import type { MobileQuickPhrase } from "../types.ts";
import { escapeAttr, escapeHtml } from "../utils.ts";

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

export const MAX_MOBILE_QUICK_PHRASES = 24;
export const MAX_MOBILE_QUICK_PHRASE_LABEL = 32;
export const MAX_MOBILE_QUICK_PHRASE_TEXT = 256;
export const MAX_MOBILE_QUICK_PHRASE_GROUP = 24;

export function renderMobileQuickPhraseKeyboardPanel(phrases: MobileQuickPhrase[]): string {
  return sortedMobileQuickPhrases(phrases).map((phrase) => (
    `<button type="button" data-mobile-phrase="${escapeAttr(phrase.id)}" title="${escapeAttr(phrase.text)}"${phrase.sendEnter ? ' data-mobile-phrase-enter="true"' : ""}>${escapeHtml(phrase.label || phrase.text)}${phrase.sendEnter ? '<span class="mobile-phrase-enter" aria-hidden="true">↵</span>' : ""}</button>`
  )).join("");
}

export function renderMobileQuickPhraseList(phrases: MobileQuickPhrase[], tr: Translate): string {
  const sorted = sortedMobileQuickPhrases(phrases);
  if (!sorted.length) {
    return `<p class="empty">${escapeHtml(tr("status.noQuickPhrases"))}</p>`;
  }
  return sorted.map((phrase, index) => `
    <div class="quick-phrase-row" data-quick-phrase-row="${escapeAttr(phrase.id)}">
      <button type="button" class="quick-phrase-preview" data-quick-phrase-edit="${escapeAttr(phrase.id)}">
        <span>${escapeHtml(phrase.label || phrase.text)}</span>
        <small>${phrase.group ? `<b>${escapeHtml(phrase.group)}</b>` : ""}${phrase.sendEnter ? '<span aria-hidden="true">↵</span>' : ""}${escapeHtml(phrase.text)}</small>
      </button>
      <div class="quick-phrase-row-actions">
        <button type="button" class="icon-button" data-quick-phrase-move="${escapeAttr(phrase.id)}" data-direction="-1" aria-label="${escapeAttr(tr("action.moveUp"))}" title="${escapeAttr(tr("action.moveUp"))}"${index === 0 ? " disabled" : ""}><i data-lucide="chevron-up"></i></button>
        <button type="button" class="icon-button" data-quick-phrase-move="${escapeAttr(phrase.id)}" data-direction="1" aria-label="${escapeAttr(tr("action.moveDown"))}" title="${escapeAttr(tr("action.moveDown"))}"${index === sorted.length - 1 ? " disabled" : ""}><i data-lucide="chevron-down"></i></button>
        <button type="button" class="icon-button" data-quick-phrase-remove="${escapeAttr(phrase.id)}" aria-label="${escapeAttr(tr("action.quickPhraseRemove"))}" title="${escapeAttr(tr("action.quickPhraseRemove"))}"><i data-lucide="trash-2"></i></button>
      </div>
    </div>
  `).join("");
}

export function sortedMobileQuickPhrases(phrases: MobileQuickPhrase[]): MobileQuickPhrase[] {
  return [...phrases].sort((left, right) => left.order - right.order);
}

export function normalizeMobileQuickPhrases(value: unknown): MobileQuickPhrase[] {
  if (!Array.isArray(value)) return [];
  const phrases: MobileQuickPhrase[] = [];
  const ids = new Set<string>();
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const raw = item as Partial<MobileQuickPhrase>;
    const text = normalizePhraseText(raw.text);
    if (!text) continue;
    const label = normalizePhraseLabel(raw.label) || text.slice(0, MAX_MOBILE_QUICK_PHRASE_LABEL);
    const id = normalizePhraseId(raw.id, ids, phrases.length);
    phrases.push({
      id,
      label,
      text,
      useCount: normalizeCount(raw.useCount),
      lastUsedAt: normalizeTimestamp(raw.lastUsedAt),
      group: normalizePhraseGroup(raw.group),
      order: normalizeOrder(raw.order, phrases.length),
      sendEnter: raw.sendEnter === true,
    });
    ids.add(id);
    if (phrases.length >= MAX_MOBILE_QUICK_PHRASES) break;
  }
  return phrases.sort((left, right) => left.order - right.order).map((phrase, order) => ({ ...phrase, order }));
}

export function makeMobileQuickPhrase(
  input: { id?: string; label: string; text: string; group?: string; sendEnter?: boolean },
  existing: MobileQuickPhrase[] = [],
): MobileQuickPhrase {
  const ids = new Set(existing.map((phrase) => phrase.id).filter((id) => id !== input.id));
  return {
    id: normalizePhraseId(input.id, ids, existing.length),
    label: normalizePhraseLabel(input.label) || normalizePhraseText(input.text).slice(0, MAX_MOBILE_QUICK_PHRASE_LABEL),
    text: normalizePhraseText(input.text),
    useCount: 0,
    lastUsedAt: 0,
    group: normalizePhraseGroup(input.group),
    order: existing.length,
    sendEnter: input.sendEnter === true,
  };
}

export function moveMobileQuickPhrase(phrases: MobileQuickPhrase[], id: string, direction: -1 | 1): MobileQuickPhrase[] {
  const next = sortedMobileQuickPhrases(normalizeMobileQuickPhrases(phrases));
  const index = next.findIndex((phrase) => phrase.id === id);
  const target = index + direction;
  if (index >= 0 && target >= 0 && target < next.length) {
    [next[index], next[target]] = [next[target], next[index]];
  }
  return next.map((phrase, order) => ({ ...phrase, order }));
}

export function markMobileQuickPhraseUsed(phrases: MobileQuickPhrase[], id: string, now = Date.now()): MobileQuickPhrase[] {
  return phrases.map((phrase) => phrase.id === id
    ? {
        ...phrase,
        useCount: Math.min(Number.MAX_SAFE_INTEGER, phrase.useCount + 1),
        lastUsedAt: now,
      }
    : phrase);
}

function normalizePhraseText(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/\r\n/g, "\n").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "").trim().slice(0, MAX_MOBILE_QUICK_PHRASE_TEXT);
}

function normalizePhraseLabel(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/\s+/g, " ").trim().slice(0, MAX_MOBILE_QUICK_PHRASE_LABEL);
}

function normalizePhraseGroup(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/\s+/g, " ").trim().slice(0, MAX_MOBILE_QUICK_PHRASE_GROUP);
}

function normalizeOrder(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.trunc(value))
    : fallback;
}

function normalizePhraseId(value: unknown, used: Set<string>, index: number): string {
  const candidate = typeof value === "string" ? value.trim().replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 64) : "";
  if (candidate && !used.has(candidate)) return candidate;
  let id = `phrase-${index + 1}`;
  let suffix = 2;
  while (used.has(id)) {
    id = `phrase-${index + suffix}`;
    suffix += 1;
  }
  return id;
}

function normalizeCount(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.trunc(value))
    : 0;
}

function normalizeTimestamp(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.trunc(value))
    : 0;
}
