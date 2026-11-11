import type { PendingDevice } from "./api.ts";
import type { SessionChoice } from "./model.ts";
import { renderPending, type Translate } from "./settings-view.ts";

/** Keep approval drafts and keyboard focus when language/session choices change. */
export function updatePending(container: HTMLElement, pending: PendingDevice[], sessions: SessionChoice[], tr: Translate) {
  const drafts = Array.from(container.querySelectorAll<HTMLFormElement>("[data-pending]")).map(form => ({
    id: form.dataset.pending,
    focusedButton: Array.from(form.querySelectorAll("button")).indexOf(document.activeElement as HTMLButtonElement),
    fields: Array.from(form.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input, select")).map(field => ({
      name: field.name, value: field.value, checked: field instanceof HTMLInputElement && field.checked,
      focused: document.activeElement === field,
    })),
  }));
  container.innerHTML = renderPending(pending, sessions, tr);
  for (const form of container.querySelectorAll<HTMLFormElement>("[data-pending]")) {
    const draft = drafts.find(item => item.id === form.dataset.pending);
    for (const field of form.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input, select")) {
      const previous = draft?.fields.find(item => item.name === field.name && (field.type !== "checkbox" || item.value === field.value));
      if (!previous) continue;
      if (field instanceof HTMLInputElement && field.type === "checkbox") field.checked = previous.checked;
      else field.value = previous.value;
      if (field.name.startsWith("root:")) {
        field.disabled = !Array.from(form.querySelectorAll<HTMLInputElement>('[name="session"]')).some(item => item.value === field.name.slice(5) && item.checked);
      }
      if (previous.focused) field.focus();
    }
    if (draft && draft.focusedButton >= 0) form.querySelectorAll("button")[draft.focusedButton]?.focus();
  }
}
