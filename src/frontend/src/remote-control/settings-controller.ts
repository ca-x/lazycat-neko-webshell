import "./settings.css";
import { remoteApi, RemoteApiError, type Invitation, type Permission, type RemoteAction, type RemoteState } from "./api.ts";
import { freshRtt, remoteStatus, unknownOperationCount, type SessionChoice } from "./model.ts";
import { renderInvitation, renderPending, type Translate } from "./settings-view.ts";
import { createDeviceList } from "./devices-view.ts";

export function createRemoteControlSettings(options: {
  root: HTMLElement; tr: Translate; sessions: () => SessionChoice[];
  api?: typeof remoteApi; now?: () => number; confirm?: (message: string) => boolean;
}) {
  const { root, tr } = options;
  const api = options.api ?? remoteApi;
  const now = options.now ?? Date.now;
  const find = <T extends HTMLElement>(selector: string) => root.querySelector<T>(selector)!;
  const config = find<HTMLFormElement>("[data-remote-config]");
  const enabled = config.elements.namedItem("enabled") as HTMLInputElement;
  const relay = config.elements.namedItem("relay") as HTMLInputElement;
  const registrationKey = config.elements.namedItem("registrationKey") as HTMLInputElement;
  const clearRegistrationKey = config.elements.namedItem("clearRegistrationKey") as HTMLInputElement;
  const invitationView = find<HTMLElement>("[data-remote-invitation]");
  const deviceList = createDeviceList(find<HTMLElement>("[data-remote-devices]"), tr);
  let state: RemoteState | undefined;
  let receivedAt = -Infinity;
  let busy = false;
  let loading = false;
  let dirty = false;
  let disposed = false;
  let invitation: (Invitation & { expiresAt: number }) | undefined;
  let pendingSignature = "";
  let deviceSignature = "";
  const deadlines = new Map<string, number>();
  let language = tr("remote.title");

  function visible() { return !document.hidden && root.checkVisibility(); }
  function error(message = "") {
    const target = find<HTMLElement>("[data-remote-error]");
    target.textContent = message;
    target.hidden = !message;
  }
  function clearInvitation() {
    invitation = undefined;
    invitationView.replaceChildren();
    invitationView.hidden = true;
  }
  function clearAuthorization() {
    state = undefined;
    receivedAt = -Infinity;
    dirty = false;
    pendingSignature = "";
    deviceSignature = "";
    deadlines.clear();
    clearInvitation();
    config.reset();
    relay.value = "";
    registrationKey.value = "";
    deviceList.clear();
    for (const selector of ["[data-remote-pending]", "[data-remote-host-fingerprint]", "[data-remote-operation-warning]"]) find<HTMLElement>(selector).replaceChildren();
    for (const selector of ["[data-remote-pairing]", "[data-remote-host-fingerprint]", "[data-remote-operation-warning]"]) find<HTMLElement>(selector).hidden = true;
    find<HTMLElement>("[data-remote-registration-help]").textContent = "";
    find<HTMLElement>("[data-remote-state]").textContent = tr("remote.unavailable");
    find<HTMLElement>("[data-remote-relay-rtt]").textContent = `${tr("remote.hostRtt")}: ${tr("remote.unavailable")}`;
  }
  function reportFailure(message: string, cause: unknown) {
    if (disposed) return;
    if (cause instanceof RemoteApiError && cause.authorizationLost) clearAuthorization();
    error(`${message}: ${String(cause instanceof Error ? cause.message : cause)}`);
  }
  function controls() {
    root.setAttribute("aria-busy", String(busy || loading));
    const working = find<HTMLElement>("[data-remote-working]");
    working.hidden = !busy && !loading;
    if (busy || loading) working.textContent = tr(loading ? "remote.loading" : "remote.working");
    for (const element of Array.from(config.elements)) {
      if (element instanceof HTMLInputElement || element instanceof HTMLButtonElement) element.disabled = busy || !state || (loading && element instanceof HTMLButtonElement);
    }
    find<HTMLButtonElement>("[data-remote-invite]").disabled = busy || loading || !state?.enabled || state.status !== "online" || now() - receivedAt >= 30000;
    find<HTMLButtonElement>("[data-remote-refresh]").disabled = busy || loading;
    const copy = root.querySelector<HTMLButtonElement>("[data-remote-copy]");
    if (copy) copy.disabled = busy || loading || !state;
    for (const element of root.querySelectorAll<HTMLButtonElement>(".remote-device button")) {
      const pending = element.closest<HTMLElement>("[data-pending]");
      const device = state?.devices.find(item => item.id === element.dataset.disconnect);
      element.disabled = busy || loading || !state
        || Boolean(pending && ((deadlines.get(pending.dataset.pending!) ?? 0) <= now() || (!options.sessions().length && element.type === "submit")))
        || Boolean(element.dataset.disconnect && !device?.online);
    }
  }
  function metrics() {
    if (!state) return;
    const stale = now() - receivedAt >= 30000;
    const status = stale && state.enabled ? "offline" : remoteStatus(state);
    const text = (element: HTMLElement, value: string) => { if (element.textContent !== value) element.textContent = value; };
    text(find<HTMLElement>("[data-remote-state]"), tr(`remote.state.${status}`));
    const format = (value: number | null) => value === null ? tr("remote.unavailable") : `${value} ms`;
    text(find<HTMLElement>("[data-remote-relay-rtt]"), `${tr("remote.hostRtt")}: ${format(freshRtt(state.relayRttMs, state.enabled && status === "online", receivedAt, now()))}`);
    for (const row of root.querySelectorAll<HTMLElement>("[data-device]")) {
      const device = state.devices.find(item => item.id === row.dataset.device);
      if (!device) continue;
      const online = !stale && state.enabled && status === "online" && device.online;
      row.querySelector<HTMLElement>("[data-device-status]")!.textContent = tr(online ? "remote.connected" : "remote.disconnected", { count: device.connections });
      row.querySelector<HTMLElement>("[data-device-rtt]")!.textContent = `${tr("remote.deviceRtt")}: ${format(freshRtt(device.rttMs, online, receivedAt, now()))}`;
    }
    for (const row of root.querySelectorAll<HTMLElement>("[data-pending]")) {
      const remaining = Math.max(0, Math.ceil(((deadlines.get(row.dataset.pending!) ?? 0) - now()) / 1000));
      row.querySelector<HTMLElement>("[data-pending-expiry]")!.textContent = remaining ? tr("remote.expires", { seconds: remaining }) : tr("remote.expired");
      if (!remaining) for (const button of row.querySelectorAll("button")) button.disabled = true;
    }
    if (invitation) {
      const remaining = Math.max(0, Math.ceil((invitation.expiresAt - now()) / 1000));
      if (!remaining || stale || !state.enabled || status !== "online") {
        clearInvitation();
        invitationView.hidden = false;
        invitationView.textContent = tr("remote.expired");
      } else find<HTMLElement>("[data-remote-invite-expiry]").textContent = tr("remote.expires", { seconds: remaining });
    }
    controls();
  }
  function apply(next: RemoteState, fresh = true) {
    if (disposed) return;
    const configurationChanged = state && (state.relay !== next.relay || state.enabled !== next.enabled);
    const newlyPending = next.pending.some(device => !state?.pending.some(previous => previous.id === device.id));
    state = next;
    const hostFingerprint = find<HTMLElement>("[data-remote-host-fingerprint]");
    hostFingerprint.hidden = !next.hostFingerprint;
    hostFingerprint.textContent = next.hostFingerprint ? tr("remote.hostFingerprint", { fingerprint: next.hostFingerprint }) : "";
    const unknown = unknownOperationCount(next);
    const operationWarning = find<HTMLElement>("[data-remote-operation-warning]");
    operationWarning.hidden = unknown === 0;
    operationWarning.textContent = unknown ? tr("remote.operationUnknown", { count: unknown }) : "";
    if (fresh) receivedAt = now();
    if (!dirty) { enabled.checked = next.enabled; relay.value = next.relay; }
    find<HTMLElement>("[data-remote-registration-help]").textContent = tr(next.has_registration_key ? "remote.registrationSaved" : "remote.registrationHelp");
    if (configurationChanged || !next.enabled || next.status !== "online" || newlyPending) clearInvitation();
    find<HTMLElement>("[data-remote-pairing]").hidden = !next.enabled;
    const sessions = options.sessions();
    const nextPendingSignature = JSON.stringify([next.pending.map(({ expiresInSeconds, ...device }) => device), sessions, tr("remote.title")]);
    if (fresh) {
      for (const id of deadlines.keys()) if (!next.pending.some(device => device.id === id)) deadlines.delete(id);
      for (const device of next.pending) deadlines.set(device.id, now() + device.expiresInSeconds * 1000);
    }
    if (pendingSignature !== nextPendingSignature) {
      find<HTMLElement>("[data-remote-pending]").innerHTML = renderPending(next.pending, sessions, tr);
      pendingSignature = nextPendingSignature;
    }
    const nextDeviceSignature = JSON.stringify([next.devices.map(({ rttMs, connections, ...device }) => device), tr("remote.title")]);
    if (deviceSignature !== nextDeviceSignature) {
      deviceList.render(next.devices);
      deviceSignature = nextDeviceSignature;
    }
    error(next.error);
    metrics();
  }
  async function refresh() {
    if (disposed || busy || loading || !visible()) return;
    loading = true;
    controls();
    try { apply(await api.get()); }
    catch (failure) {
      receivedAt = -Infinity;
      clearInvitation();
      if (!disposed) { reportFailure(tr("remote.loadFailed"), failure); metrics(); }
    } finally { loading = false; if (!disposed) controls(); }
  }
  async function mutate(action: RemoteAction | { action: "invite" }) {
    if (busy || loading || disposed) return;
    busy = true;
    controls();
    const focus = document.activeElement;
    const focusId = focus instanceof HTMLElement ? focus.closest<HTMLElement>("[data-pending], [data-device]")?.dataset : undefined;
    try {
      error();
      if (action.action === "invite") {
        clearInvitation();
        const result = await api.invite();
        if (disposed || !visible()) return;
        invitation = { ...result, expiresAt: now() + result.expiresInSeconds * 1000 };
        invitationView.innerHTML = renderInvitation(result.code, tr);
        invitationView.hidden = false;
      } else {
        if (action.action === "configure") clearInvitation();
        const result = await api.act(action);
        if (action.action === "configure") { dirty = false; registrationKey.value = ""; clearRegistrationKey.checked = false; }
        pendingSignature = "";
        deviceSignature = "";
        apply(result);
      }
    } catch (failure) {
      reportFailure(tr("remote.actionFailed"), failure);
    } finally {
      busy = false;
      if (!disposed) {
        controls();
        if (focus instanceof HTMLElement && !focus.isConnected) {
          const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-pending], [data-device]"));
          const row = rows.find(item => focusId?.device && item.dataset.device === focusId.device || focusId?.pending && item.dataset.pending === focusId.pending);
          (row?.querySelector<HTMLElement>("button, select") ?? find<HTMLElement>("[data-remote-refresh]")).focus();
        }
      }
    }
  }
  function onInput(event: Event) {
    const input = event.target;
    if (!(input instanceof HTMLInputElement)) return;
    if (config.contains(input)) dirty = true;
    if (input.name === "session") {
      const form = input.closest("form")!;
      const rootInput = form.elements.namedItem(`root:${input.value}`) as HTMLInputElement;
      rootInput.disabled = !input.checked;
    }
  }
  function onSubmit(event: SubmitEvent) {
    const form = event.target;
    if (!(form instanceof HTMLFormElement)) return;
    event.preventDefault();
    if (busy || loading || !state) return;
    if (form === config) {
      void mutate({ action: "configure", enabled: enabled.checked, relay: relay.value.trim(), registrationKey: clearRegistrationKey.checked ? "" : registrationKey.value || undefined });
      return;
    }
    if (!form.dataset.pending) return;
    const data = new FormData(form);
    const resources = data.getAll("session").map(value => ({ session: String(value), root: String(data.get(`root:${value}`) ?? "").trim() || null }));
    if (!resources.length || resources.length > 16) { error(tr("remote.chooseResources")); return; }
    void mutate({ action: "approve", pending: form.dataset.pending, permission: data.get("permission") as Permission, resources });
  }
  function onClick(event: MouseEvent) {
    const button = (event.target as Element).closest<HTMLButtonElement>("button");
    if (!button || busy || loading) return;
    if (button.hasAttribute("data-remote-refresh")) { void refresh(); return; }
    if (!state || button.disabled) return;
    if (button.hasAttribute("data-remote-invite")) { void mutate({ action: "invite" }); return; }
    if (button.hasAttribute("data-remote-copy") && invitation) {
      void navigator.clipboard.writeText(invitation.code).then(() => { if (!disposed) button.textContent = tr("remote.copied"); }, () => error(tr("remote.copyFailed")));
      return;
    }
    if (button.dataset.reject) { void mutate({ action: "reject", pending: button.dataset.reject }); return; }
    const id = button.dataset.permission ?? button.dataset.disconnect ?? button.dataset.revoke;
    const device = state?.devices.find(item => item.id === id);
    if (!device) return;
    if (button.dataset.permission) {
      const permission = button.closest("article")!.querySelector<HTMLSelectElement>("select")!.value as Permission;
      if (permission === device.permission) return;
      if (permission === "operate" && !(options.confirm ?? window.confirm)(tr("remote.confirmOperate", { name: device.name }))) return;
      void mutate({ action: "permission", id: device.id, permission });
    } else {
      const action = button.dataset.revoke ? "revoke" : "disconnect";
      if (!(options.confirm ?? window.confirm)(tr(action === "revoke" ? "remote.confirmRevoke" : "remote.confirmDisconnect", { name: device.name }))) return;
      void mutate({ action, id: device.id });
    }
  }
  root.addEventListener("input", onInput);
  root.addEventListener("submit", onSubmit);
  root.addEventListener("click", onClick);
  const timer = window.setInterval(() => {
    if (!visible()) { clearInvitation(); return; }
    if (language !== tr("remote.title")) { language = tr("remote.title"); pendingSignature = ""; deviceSignature = ""; if (state) apply(state, false); }
    metrics();
    if (now() - receivedAt >= 2500) void refresh();
  }, 1000);
  return {
    refresh,
    dispose() {
      disposed = true;
      clearInvitation();
      deviceList.clear();
      registrationKey.value = "";
      window.clearInterval(timer);
      root.removeEventListener("input", onInput);
      root.removeEventListener("submit", onSubmit);
      root.removeEventListener("click", onClick);
    },
  };
}
