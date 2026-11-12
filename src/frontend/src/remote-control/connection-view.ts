import type { ConnectionKind } from "./api.ts";
import type { Translate } from "./settings-view.ts";

// Lucide-style symbols inherit the current theme's foreground color.
const paths = {
  direct: '<path d="m10 13 4-4m-6 7-1 1a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0m2 1 1-1a4 4 0 0 1 6 6l-4 4a4 4 0 0 1-6 0"/>',
  relay: '<path d="M20 16.6A5 5 0 0 0 18 7h-1.3a7 7 0 1 0-13.6 6M12 12v9m-3-3 3 3 3-3"/>',
  connecting: '<path d="M12 3a9 9 0 1 1-9 9"/>',
};

export function updateConnectionView(root: HTMLElement, kinds: ConnectionKind[], online: boolean, tr: Translate) {
  const current = online ? [...new Set(kinds.filter(kind => kind !== "local"))] : [];
  const label = current.length ? current.map(kind => tr(`remote.path.${kind}`)).join(" · ") : tr("remote.disconnected");
  const summary = root.querySelector<HTMLElement>("summary")!;
  summary.title = label;
  summary.setAttribute("aria-label", label);
  root.querySelector<HTMLElement>("[data-connection-detail]")!.textContent = label;
  const icon = current.includes("direct") && !current.includes("relay") ? "direct" : current.includes("connecting") && !current.includes("relay") ? "connecting" : "relay";
  const svg = root.querySelector<SVGElement>("svg")!;
  if (svg.dataset.kind !== icon) { svg.innerHTML = paths[icon]; svg.dataset.kind = icon; }
  root.dataset.online = String(online);
}

export const connectionView = `<details class="remote-path" data-device-path><summary><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"></svg></summary><span data-connection-detail></span></details>`;
