import type { RemoteDevice } from "./api.ts";
import { renderDevices, type Translate } from "./settings-view.ts";

export function createDeviceList(container: HTMLElement, tr: Translate) {
  const rows = new Map<string, { element: HTMLElement; signature: string; permission: RemoteDevice["permission"] }>();
  return {
    clear() {
      rows.clear();
      container.replaceChildren();
    },
    render(devices: RemoteDevice[]) {
      for (const [id, row] of rows) {
        if (devices.some(device => device.id === id)) continue;
        row.element.remove();
        rows.delete(id);
      }
      if (!devices.length) {
        container.innerHTML = renderDevices([], tr);
        return;
      }
      if (!rows.size) container.replaceChildren();
      for (const [index, device] of devices.entries()) {
        const { rttMs, connections, online, ...content } = device;
        const signature = JSON.stringify([content, tr("remote.title")]);
        const previous = rows.get(device.id);
        let element = previous?.element;
        if (!previous || previous.signature !== signature) {
          const template = document.createElement("template");
          template.innerHTML = renderDevices([device], tr);
          element = template.content.firstElementChild as HTMLElement;
          const interactive = "summary, select, button";
          const focusIndex = previous ? Array.from(previous.element.querySelectorAll(interactive)).indexOf(document.activeElement!) : -1;
          if (previous) {
            element.querySelector<HTMLDetailsElement>("details")!.open = previous.element.querySelector<HTMLDetailsElement>("details")!.open;
            if (previous.permission === device.permission) element.querySelector<HTMLSelectElement>("select")!.value = previous.element.querySelector<HTMLSelectElement>("select")!.value;
            previous.element.replaceWith(element);
          }
          rows.set(device.id, { element, signature, permission: device.permission });
          if (container.children[index] !== element) container.insertBefore(element, container.children[index] ?? null);
          if (focusIndex >= 0) element.querySelectorAll<HTMLElement>(interactive)[focusIndex]?.focus();
        } else if (container.children[index] !== element) {
          container.insertBefore(element!, container.children[index] ?? null);
        }
      }
    },
  };
}
