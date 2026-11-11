import type { MessageKey } from "../i18n";
import { escapeHtml } from "../utils.ts";
import type { PendingDevice, RemoteDevice } from "./api.ts";
import type { SessionChoice } from "./model.ts";
import { invitationQr } from "./qr.ts";

export type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;
export function renderRemoteControlSettings(): string {
  return `<section class="settings-section remote-control" id="remoteControlSettingsPanel" data-settings-panel="remote-control" role="tabpanel" aria-labelledby="settings-tab-remote-control" hidden>
    <header class="settings-panel-intro"><h3 data-i18n="remote.title"></h3><p data-i18n="settings.description.remote-control"></p></header>
    <form data-remote-config>
      <label class="switch"><input type="checkbox" name="enabled"><span data-i18n="remote.enabled"></span></label>
      <p class="settings-help" data-i18n="remote.enabledHelp"></p>
      <label class="remote-field"><span data-i18n="remote.relay"></span><input name="relay" type="url" inputmode="url" autocomplete="off" spellcheck="false" placeholder="https://relay.example"></label>
      <button class="command-button" type="button" data-remote-public-relay data-i18n="remote.usePublicRelay"></button>
      <p class="settings-help" data-i18n="remote.relayHelp"></p>
      <details data-remote-connection-options><summary data-i18n="remote.moreConnectionOptions"></summary>
        <label class="remote-field"><span data-i18n="remote.registrationKey"></span><input name="registrationKey" type="password" autocomplete="new-password" spellcheck="false" maxlength="256"></label>
        <p class="settings-help" data-remote-registration-help></p>
        <label><input name="clearRegistrationKey" type="checkbox"> <span data-i18n="remote.clearRegistrationKey"></span></label>
      </details>
      <button type="submit" class="command-button remote-primary" data-remote-save data-i18n="remote.save"></button>
    </form>
    <div class="remote-connection" role="status" aria-live="polite"><strong data-remote-state></strong><span data-remote-relay-rtt></span></div>
    <p class="settings-help" data-i18n="remote.rttHelp"></p>
    <p class="remote-fingerprint" data-remote-host-fingerprint hidden></p>
    <p class="settings-help" data-remote-operation-warning role="status" hidden></p>
    <p data-remote-error role="alert" hidden></p>
    <p class="settings-help" data-remote-working role="status" hidden></p>
    <button class="command-button" type="button" data-remote-refresh data-i18n="remote.refresh"></button>
    <section class="remote-section" data-remote-pairing hidden><h4 data-i18n="remote.pair"></h4>
      <p class="settings-help" data-i18n="remote.pairHelp"></p>
      <button class="command-button" type="button" data-remote-invite data-i18n="remote.invite" disabled></button>
      <div data-remote-invitation hidden></div>
    </section>
    <details class="remote-section" data-remote-pending-section><summary><span data-i18n="remote.pending"></span> <span data-remote-pending-count></span></summary><p class="settings-help" data-i18n="remote.scopeHelp"></p><div data-remote-pending></div></details>
    <details class="remote-section" data-remote-devices-section><summary><span data-i18n="remote.devices"></span> <span data-remote-device-count></span></summary><p class="settings-help" data-i18n="remote.permissionHelp"></p><div data-remote-devices></div></details>
  </section>`;
}

export function permissionSelect(tr: Translate, name: string, value = "read_only"): string {
  return `<label class="remote-field"><span>${escapeHtml(tr("remote.permission"))}</span><select name="${name}"><option value="read_only" ${value === "read_only" ? "selected" : ""}>${escapeHtml(tr("remote.readOnly"))}</option><option value="operate" ${value === "operate" ? "selected" : ""}>${escapeHtml(tr("remote.operate"))}</option></select></label>`;
}

export function renderPending(pending: PendingDevice[], sessions: SessionChoice[], tr: Translate): string {
  if (!pending.length) return `<p class="settings-help">${escapeHtml(tr("remote.noPending"))}</p>`;
  return pending.map(device => `<form class="remote-device" data-pending="${escapeHtml(device.id)}">
    <h5>${escapeHtml(device.name)}</h5><p class="remote-fingerprint">${escapeHtml(tr("remote.fingerprint"))}: ${escapeHtml(device.fingerprint)}</p>
    <p data-pending-expiry="${escapeHtml(device.id)}"></p>
    <fieldset><legend>${escapeHtml(tr("remote.resources"))}</legend>${sessions.length ? sessions.map(session => `<div class="remote-resource">
      <label><input type="checkbox" name="session" value="${escapeHtml(session.id)}"> <span>${escapeHtml(session.name)} <small>${escapeHtml(session.selector)} · ${escapeHtml(session.backend)}</small></span></label>
      <label class="remote-field"><span>${escapeHtml(tr("remote.root"))}</span><input name="root:${escapeHtml(session.id)}" autocomplete="off" placeholder="/home/user" disabled></label>
    </div>`).join("") : `<p>${escapeHtml(tr("remote.noResources"))}</p>`}</fieldset>
    ${permissionSelect(tr, "permission")}
    <div class="remote-actions"><button type="submit" class="command-button remote-primary" ${sessions.length ? "" : "disabled"}>${escapeHtml(tr("remote.approve"))}</button><button class="command-button" type="button" data-reject="${escapeHtml(device.id)}">${escapeHtml(tr("remote.reject"))}</button></div>
  </form>`).join("");
}

export function renderDevices(devices: RemoteDevice[], tr: Translate): string {
  if (!devices.length) return `<p class="settings-help">${escapeHtml(tr("remote.noDevices"))}</p>`;
  return devices.map(device => `<article class="remote-device" data-device="${escapeHtml(device.id)}">
    <h5>${escapeHtml(device.name)}</h5>
    <p data-device-status></p><p data-device-rtt></p>
    <p>${escapeHtml(tr("remote.currentPermission"))}: ${escapeHtml(tr(device.permission === "operate" ? "remote.operate" : "remote.readOnly"))}</p>
    <details data-device-management><summary>${escapeHtml(tr("remote.manageDevice"))}</summary>
    <p class="remote-fingerprint">${escapeHtml(tr("remote.fingerprint"))}: ${escapeHtml(device.fingerprint)}</p>
    <ul>${device.resources.map(resource => `<li>${escapeHtml(resource.name)} <small>${escapeHtml(resource.backend)} · ${escapeHtml(tr(resource.files ? "remote.withFiles" : "remote.noFiles"))}</small></li>`).join("")}</ul>
    ${permissionSelect(tr, "permission", device.permission)}
    <div class="remote-actions"><button class="command-button" type="button" data-permission="${escapeHtml(device.id)}">${escapeHtml(tr("remote.updatePermission"))}</button><button class="command-button" type="button" data-disconnect="${escapeHtml(device.id)}" ${device.online ? "" : "disabled"}>${escapeHtml(tr("remote.disconnect"))}</button><button class="command-button" type="button" data-revoke="${escapeHtml(device.id)}">${escapeHtml(tr("remote.revoke"))}</button></div>
    </details>
  </article>`).join("");
}

export function renderInvitation(code: string, tr: Translate): string {
  return `<div role="img" aria-label="${escapeHtml(tr("remote.qr"))}">${invitationQr(code)}</div>
    <label class="remote-field"><span>${escapeHtml(tr("remote.manual"))}</span><textarea readonly rows="4" spellcheck="false">${escapeHtml(code)}</textarea></label>
    <p data-remote-invite-expiry></p><button class="command-button" type="button" data-remote-copy>${escapeHtml(tr("remote.copy"))}</button>`;
}
