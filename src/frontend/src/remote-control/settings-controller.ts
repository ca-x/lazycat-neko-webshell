import "./settings.css";
import { checkRelayAddress, RelayCheckError } from "./relay-check.ts";
import { remoteApi, RemoteApiError, type Invitation, type Permission, type RemoteAction, type RemoteState } from "./api.ts";
import { freshRtt, remoteStatus, unknownOperationCount, type SessionChoice } from "./model.ts";
import { renderInvitation, type Translate } from "./settings-view.ts";
import { updatePending } from "./pending-view.ts";
import { createDeviceList } from "./devices-view.ts";

export function createRemoteControlSettings(options: {
  root: HTMLElement; tr: Translate; sessions: () => SessionChoice[];
  checkRelay?: (relay: string) => Promise<void>;
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
  let readGeneration = 0;
  let checkingRelay = false;
  let draftRevision = 0;
  const dirty = { enabled: false, relay: false };
  let polling = false;
  let nextPollAt = Infinity;
  let pollFailures = 0;
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
    for (const selector of ["[data-remote-pending-count]", "[data-remote-device-count]"]) find<HTMLElement>(selector).textContent = "";
    receivedAt = -Infinity;
    polling = false;
    pendingSignature = "";
    deviceSignature = "";
    deadlines.clear();
    clearInvitation();
    if (!dirty.enabled) enabled.checked = false;
    if (!dirty.relay) relay.value = "";
    clearRegistrationKey.checked = false;
    registrationKey.value = "";
    deviceList.clear();
    for (const selector of ["[data-remote-pending]", "[data-remote-host-fingerprint]", "[data-remote-operation-warning]"]) find<HTMLElement>(selector).replaceChildren();
    for (const selector of ["[data-remote-pairing]", "[data-remote-host-fingerprint]", "[data-remote-operation-warning]"]) find<HTMLElement>(selector).hidden = true;
    find<HTMLElement>("[data-remote-registration-help]").textContent = "";
    find<HTMLElement>("[data-remote-state]").textContent = tr("remote.unavailable");
    find<HTMLElement>("[data-remote-relay-rtt]").textContent = `${tr("remote.hostRtt")}: ${tr("remote.unavailable")}`;
  }
  function schedulePoll(failed = false) {
    pollFailures = failed ? Math.min(pollFailures + 1, 4) : 0;
    nextPollAt = polling ? now() + Math.min(30000, 2500 * 2 ** pollFailures) : Infinity;
  }
  function reportFailure(message: string, cause: unknown) {
    if (disposed) return;
    if (cause instanceof RemoteApiError && cause.authorizationLost) clearAuthorization();
    else schedulePoll(true);
    error(cause instanceof RelayCheckError ? tr(`remote.relayCheck.${cause.reason}`) : `${message}: ${String(cause instanceof Error ? cause.message : cause)}`);
  }
  function controls() {
    enabled.indeterminate = !state && !dirty.enabled;
    root.setAttribute("aria-busy", String(busy || loading));
    const working = find<HTMLElement>("[data-remote-working]");
    working.hidden = !busy && !loading;
    if (busy || loading) working.textContent = tr(loading ? "remote.loading" : checkingRelay ? "remote.checkingRelay" : "remote.working");
    for (const element of Array.from(config.elements)) {
      if (element instanceof HTMLInputElement) element.disabled = busy && !checkingRelay;
      if (element instanceof HTMLButtonElement) element.disabled = busy && (!checkingRelay || !element.hasAttribute("data-remote-public-relay"));
    }
    find<HTMLButtonElement>("[data-remote-invite]").disabled = busy || !state?.enabled || state.status !== "online" || now() - receivedAt >= 30000;
    find<HTMLButtonElement>("[data-remote-refresh]").disabled = busy || loading;
    const copy = root.querySelector<HTMLButtonElement>("[data-remote-copy]");
    if (copy) copy.disabled = busy || !state;
    for (const element of root.querySelectorAll<HTMLButtonElement>(".remote-device button")) {
      const pending = element.closest<HTMLElement>("[data-pending]");
      const device = state?.devices.find(item => item.id === element.dataset.disconnect);
      element.disabled = busy || !state
        || Boolean(pending && (!state?.enabled || (deadlines.get(pending.dataset.pending!) ?? 0) <= now() || (!options.sessions().length && element.type === "submit")))
        || Boolean(element.dataset.disconnect && (!state?.enabled || !device?.online));
    }
  }
  function metrics() {
    if (!state) {
      find<HTMLElement>("[data-remote-state]").textContent = tr("remote.notLoaded");
      find<HTMLElement>("[data-remote-relay-rtt]").textContent = "";
      controls();
      return;
    }
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
    if (fresh) { receivedAt = now(); polling = next.enabled && !!next.relay; schedulePoll(Boolean(next.error)); }
    if (!dirty.enabled) enabled.checked = next.enabled;
    if (!dirty.relay) relay.value = next.relay;
    find<HTMLElement>("[data-remote-registration-help]").textContent = tr(next.has_registration_key ? "remote.registrationSaved" : "remote.registrationHelp");
    if (configurationChanged || !next.enabled || next.status !== "online" || newlyPending) clearInvitation();
    find<HTMLElement>("[data-remote-pairing]").hidden = !next.enabled;
    find<HTMLElement>("[data-remote-pending-count]").textContent = String(next.pending.length);
    find<HTMLElement>("[data-remote-device-count]").textContent = String(next.devices.length);
    const sessions = options.sessions();
    const nextPendingSignature = JSON.stringify([next.pending.map(({ expiresInSeconds, ...device }) => device), sessions, tr("remote.title")]);
    if (fresh) {
      for (const id of deadlines.keys()) if (!next.pending.some(device => device.id === id)) deadlines.delete(id);
      for (const device of next.pending) deadlines.set(device.id, now() + device.expiresInSeconds * 1000);
    }
    if (pendingSignature !== nextPendingSignature) {
      updatePending(find<HTMLElement>("[data-remote-pending]"), next.pending, sessions, tr);
      pendingSignature = nextPendingSignature;
    }
    const nextDeviceSignature = JSON.stringify([next.devices.map(({ rttMs, connections, ...device }) => device), tr("remote.title")]);
    if (deviceSignature !== nextDeviceSignature) {
      deviceList.render(next.devices);
      deviceSignature = nextDeviceSignature;
    }
    if (fresh) error(next.error);
    metrics();
  }
  async function refresh() {
    if (disposed || busy || loading || !visible()) return;
    const generation = ++readGeneration;
    loading = true;
    controls();
    try {
      const next = await api.get();
      if (generation === readGeneration) apply(next);
    } catch (failure) {
      if (disposed || generation !== readGeneration) return;
      receivedAt = -Infinity;
      clearInvitation();
      reportFailure(tr("remote.loadFailed"), failure);
      metrics();
    } finally {
      if (generation === readGeneration) { loading = false; if (!disposed) controls(); }
    }
  }
  async function mutate(action: RemoteAction | { action: "invite" }) {
    if (busy || disposed) return;
    // Mutations supersede background reads, including their errors and finally handlers.
    readGeneration++;
    loading = false;
    const revision = draftRevision;
    busy = true;
    checkingRelay = action.action === "configure" && action.enabled;
    controls();
    const focus = document.activeElement;
    const focusId = focus instanceof HTMLElement ? focus.closest<HTMLElement>("[data-pending], [data-device]")?.dataset : undefined;
    try {
      error();
      if (action.action === "configure" && action.enabled) {
        await (options.checkRelay ?? checkRelayAddress)(action.relay);
        if (disposed) return;
        if (revision !== draftRevision) {
          error(tr("remote.configurationChanged"));
          return;
        }
        checkingRelay = false;
        controls();
      }
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
        if (action.action === "configure") { dirty.enabled = dirty.relay = false; registrationKey.value = ""; clearRegistrationKey.checked = false; }
        pendingSignature = "";
        deviceSignature = "";
        apply(result);
      }
    } catch (failure) {
      reportFailure(tr("remote.actionFailed"), failure);
    } finally {
      checkingRelay = false;
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
    if (config.contains(input)) {
      draftRevision++;
      if (input === enabled || input === relay) dirty[input === enabled ? "enabled" : "relay"] = true;
      if (input === enabled) enabled.indeterminate = false;
    }
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
    if (busy) return;
    if (form === config) {
      if (enabled.indeterminate) { error(tr("remote.chooseEnabled")); enabled.focus(); return; }
      void mutate({ action: "configure", enabled: enabled.checked, relay: relay.value.trim(), registrationKey: clearRegistrationKey.checked ? "" : registrationKey.value || undefined });
      return;
    }
    if (!state?.enabled || !form.dataset.pending || (deadlines.get(form.dataset.pending) ?? 0) <= now()) return;
    const data = new FormData(form);
    const resources = data.getAll("session").map(value => ({ session: String(value), root: String(data.get(`root:${value}`) ?? "").trim() || null }));
    if (!resources.length || resources.length > 16) { error(tr("remote.chooseResources")); return; }
    void mutate({ action: "approve", pending: form.dataset.pending, permission: data.get("permission") as Permission, resources });
  }
  function onClick(event: MouseEvent) {
    const button = (event.target as Element).closest<HTMLButtonElement>("button");
    if (!button || button.disabled || disposed) return;
    if (button.hasAttribute("data-remote-public-relay") && (!busy || checkingRelay)) {
      relay.value = "https://neko-relay.czyt.eu.org";
      dirty.relay = true;
      draftRevision++;
      relay.focus();
      return;
    }
    if (busy) return;
    if (button.hasAttribute("data-remote-refresh")) { void refresh(); return; }
    if (!state) return;
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
    if (polling && state?.enabled && state.relay && now() >= nextPollAt) void refresh();
  }, 1000);
  metrics();
  return {
    activate() { if (!disposed) { metrics(); return refresh(); } },
    refresh,
    dispose() {
      disposed = true;
      readGeneration++;
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
