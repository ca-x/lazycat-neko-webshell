import assert from "node:assert/strict";
import { createServer } from "vite";
import { createBrowserDriver, uniqueBrowserSession } from "../../../../tests-auto/browser-driver.mjs";

const fixture = `<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body><main id="fixture" class="webshell" data-interface-tone="dark" style="display:block;overflow:auto;padding:16px"></main><script type="module">
import '/src/styles.css';
import '/src/webshell-themes.css';
import '/src/settings/settings.css';
import { renderRemoteControlSettings } from '/src/remote-control/settings-view.ts';
import { createRemoteControlSettings } from '/src/remote-control/settings-controller.ts';
import { RelayCheckError } from '/src/remote-control/relay-check.ts';
import { remoteApi } from '/src/remote-control/api.ts';
import { translate } from '/src/i18n.ts';
window.clock = 1000;
window.actions = [];
window.gets = 0; window.healthChecks = []; window.order = [];
const originalInterval = window.setInterval;
window.setInterval = (callback, ms) => { window.tick = callback; return originalInterval(callback, ms); };
window.confirmations = [];
window.fail = false;
window.locale = 'en';
window.remote = { enabled: false, relay: '', status: 'disabled', error: '', relayRttMs: null, devices: [], pending: [] };
const tr = (key, values) => translate(window.locale, key, values);
document.querySelector('#fixture').innerHTML = renderRemoteControlSettings();
const root = document.querySelector('#remoteControlSettingsPanel'); root.hidden = false;
document.querySelectorAll('[data-i18n]').forEach(node => node.textContent = tr(node.dataset.i18n));
window.controller = createRemoteControlSettings({ root, tr, now: () => window.clock,
  sessions: () => [{ id: 'session-1', name: 'Owned terminal', selector: 'host@owner', backend: 'webshell' }],
  confirm: message => { window.confirmations.push(message); return true; },
  checkRelay: async relay => { window.healthChecks.push(relay); window.order.push('health'); if (window.deferHealth) await new Promise(resolve => window.resolveHealth = resolve); if (window.healthFailure) throw new RelayCheckError('connection'); },
  api: { get: async () => { window.gets++;  if (window.deferGet) await new Promise(resolve => window.resolveGet = resolve); if (window.authFailure) return remoteApi.get(); if (window.fail) throw Error('offline'); return structuredClone(window.remote); },
    invite: async () => window.authFailure ? remoteApi.invite() : ({ code: 'NR1.' + 'A'.repeat(43) + '.' + 'B'.repeat(43) + '.digest.check', expiresInSeconds: 300 }),
    act: async action => { window.actions.push(action); window.order.push(action.action); if (window.authFailure) return remoteApi.act(action); if (window.fail) throw Error('Rejected');
      if (action.action === 'configure') Object.assign(window.remote, {enabled: action.enabled, relay: action.relay, status: action.enabled ? 'online' : 'disabled', relayRttMs: 12, has_registration_key: !!action.registrationKey});
      if (action.action === 'approve') { window.remote.pending = []; window.remote.devices.push({id: 'device-1', name: 'Phone <test>', fingerprint: 'abc123', permission: action.permission, resources: [{id: 'grant-1', name: 'Owned terminal', backend: 'webshell', files: !!action.resources[0].root}], online: true, connections: 2, rttMs: 78}); }
      if (action.action === 'permission') window.remote.devices[0].permission = action.permission;
      if (action.action === 'reject') window.remote.pending = [];
      if (action.action === 'disconnect') window.remote.devices[0].online = false;
      if (action.action === 'revoke') window.remote.devices = [];
      return structuredClone(window.remote); }
  }
});
window.setInterval = originalInterval;
window.controller.activate(); window.ready = true;
</script></body></html>`;

const server = await createServer({ configFile: false, root: "src/frontend", server: { host: "127.0.0.1", port: 0 }, plugins: [{
  name: "remote-control-check", configureServer(server) {
    server.middlewares.use((request, response, next) => {
      if (request.url === '/api/remoted') {
        response.statusCode = 403;
        response.setHeader('content-type', 'application/json');
        response.end(JSON.stringify({code: 'AUTHORIZATION_FAILED', error: 'Owner verification failed'}));
        return;
      }
      if (request.url !== "/remote-control-check") return next();
      response.setHeader("content-type", "text/html"); response.end(fixture);
    });
  },
}] });
const browser = createBrowserDriver({ session: uniqueBrowserSession("remote-control"), headed: false });
try {
  await server.listen();
  await browser.open(`http://127.0.0.1:${server.httpServer.address().port}/remote-control-check`);
  await browser.command(["wait", "--fn", "window.ready === true"]);
  const evaluate = source => browser.evaluate(source);
  assert.equal(await evaluate(`window.gets`), 0);
  assert.match(await evaluate(`document.querySelector('[data-remote-state]').textContent`), /not loaded/);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-remote-config] input, [data-remote-config] button')).every(element => !element.disabled)`), true);
  await evaluate(`window.clock += 10000; window.tick(); window.controller.activate()`);
  assert.equal(await evaluate(`window.gets + window.healthChecks.length`), 0);
  await evaluate(`(async () => { window.fail = true; await window.controller.refresh(); window.clock += 10000; window.tick(); })()`);
  assert.equal(await evaluate(`window.gets`), 1);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').disabled || document.querySelector('[data-remote-save]').disabled`), false);
  await evaluate(`(async () => {
    window.fail = false; window.healthFailure = true;
    const form = document.querySelector('[data-remote-config]');
    form.elements.enabled.checked = true; form.elements.enabled.dispatchEvent(new Event('input', {bubbles:true}));
    form.elements.relay.value = 'https://local.example'; form.elements.relay.dispatchEvent(new Event('input', {bubbles:true}));
    form.requestSubmit(); await new Promise(resolve => setTimeout(resolve,0));
  })()`);
  assert.equal(await evaluate(`window.actions.length`), 0);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), 'https://local.example');
  assert.match(await evaluate(`document.querySelector('[data-remote-error]').textContent`), /secure WebSocket/);
  await evaluate(`(async () => {
    window.healthFailure = false; window.deferHealth = true;
    document.querySelector('[data-remote-config]').requestSubmit(); await new Promise(resolve => setTimeout(resolve,0));
  })()`);
  assert.equal(await evaluate(`window.actions.length`), 0);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').disabled`), false);
  await browser.command(["fill", "[name=relay]", "https://edited-during-health.example"]);
  await browser.command(["click", "[data-remote-public-relay]"]);
  await browser.command(["fill", "[name=relay]", "https://new-draft.example"]);
  await evaluate(`document.querySelector('[data-remote-connection-options]').open = true`);
  await browser.command(["fill", "[name=registrationKey]", "new-key-during-health"]);
  await browser.command(["click", "[name=enabled]"]);
  await browser.command(["click", "[name=enabled]"]);
  await evaluate(`(async () => { window.deferHealth = false; window.resolveHealth(); await new Promise(resolve => setTimeout(resolve,0)); })()`);
  assert.deepEqual(await evaluate(`({actions:window.actions.length, relay:document.querySelector('[name=relay]').value, key:document.querySelector('[name=registrationKey]').value, enabled:document.querySelector('[name=enabled]').checked, saveEnabled:!document.querySelector('[data-remote-save]').disabled, retryPrompt:document.querySelector('[data-remote-error]').textContent.includes('Save again')})`), {actions:0,relay:'https://new-draft.example',key:'new-key-during-health',enabled:true,saveEnabled:true,retryPrompt:true});
  await evaluate(`(async () => { document.querySelector('[data-remote-config]').requestSubmit(); await new Promise(resolve => setTimeout(resolve,0)); })()`);
  assert.deepEqual(await evaluate(`window.order`), ['health','health','health','configure']);
  assert.equal(await evaluate(`window.actions[0].enabled`), true);
  assert.equal(await evaluate(`window.gets`), 1);
  await evaluate(`(async () => {
    window.healthFailure = true; document.querySelector('[name=enabled]').checked = false;
    document.querySelector('[data-remote-config]').requestSubmit(); await new Promise(resolve => setTimeout(resolve,0));
    window.clock += 10000; window.tick();
  })()`);
  assert.equal(await evaluate(`window.healthChecks.length`), 3);
  assert.equal(await evaluate(`window.actions.at(-1).enabled`), false);
  assert.equal(await evaluate(`window.gets`), 1);
  // A GET started before editing must not replace either local field.
  await evaluate(`(() => {
    window.deferGet = true; window.refreshPromise = window.controller.refresh();
    const relay = document.querySelector('[name=relay]'); relay.value = 'https://late-draft.example'; relay.dispatchEvent(new Event('input', {bubbles:true}));
    const enabled = document.querySelector('[name=enabled]'); enabled.checked = true; enabled.dispatchEvent(new Event('input', {bubbles:true}));
  })()`);
  await evaluate(`(async () => { window.deferGet = false; window.resolveGet(); await window.refreshPromise; })()`);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), 'https://late-draft.example');
  assert.equal(await evaluate(`document.querySelector('[name=enabled]').checked`), true);
  // Successful saved state permits polling; a failed request pauses it until an explicit recovery.
  await evaluate(`(async () => {
    window.healthFailure = false; document.querySelector('[data-remote-config]').requestSubmit();
    await new Promise(resolve => setTimeout(resolve,0)); window.beforePoll = window.gets;
    window.clock += 3000; window.tick(); await new Promise(resolve => setTimeout(resolve,0));
  })()`);
  assert.equal(await evaluate(`window.gets === window.beforePoll + 1`), true);
  await evaluate(`(async () => {
    window.fail = true; window.clock += 3000; window.tick(); await new Promise(resolve => setTimeout(resolve,0));
    window.afterFailedPoll = window.gets; window.clock += 10000; window.tick();
  })()`);
  assert.equal(await evaluate(`window.gets === window.afterFailedPoll`), true);
  await evaluate(`(async () => {
    window.fail = false; await window.controller.refresh(); window.beforePoll = window.gets;
    window.clock += 3000; window.tick(); await new Promise(resolve => setTimeout(resolve,0));
  })()`);
  assert.equal(await evaluate(`window.gets === window.beforePoll + 1`), true);
  for (const stopped of [{enabled:false}, {enabled:true,relay:''}, {enabled:true,relay:'https://relay.example',error:'Relay unavailable'}]) {
    await evaluate(`(async () => { Object.assign(window.remote, ${JSON.stringify(stopped)}); await window.controller.refresh(); window.beforePoll = window.gets; window.clock += 10000; window.tick(); })()`);
    assert.equal(await evaluate(`window.gets === window.beforePoll`), true);
  }
  // The public Relay shortcut edits only the local relay draft, including keyboard/mobile use.
  await browser.command(["set", "viewport", "360", "844"]);
  await evaluate(`window.beforeShortcut = {enabled:document.querySelector('[name=enabled]').checked, calls:window.gets + window.actions.length + window.healthChecks.length, server:JSON.stringify(window.remote)}; document.querySelector('[data-remote-public-relay]').focus()`);
  await browser.press("Enter");
  assert.deepEqual(await evaluate(`(async () => {
    const result = {
      relay:document.querySelector('[name=relay]').value,
      focused:document.activeElement === document.querySelector('[name=relay]'),
      enabledUnchanged:document.querySelector('[name=enabled]').checked === window.beforeShortcut.enabled,
      noRequests:window.gets + window.actions.length + window.healthChecks.length === window.beforeShortcut.calls,
      serverUnchanged:JSON.stringify(window.remote) === window.beforeShortcut.server,
    };
    await window.controller.refresh();
    result.retainedAfterRead = document.querySelector('[name=relay]').value;
    return result;
  })()`), {relay:'https://neko-relay.czyt.eu.org',focused:true,enabledUnchanged:true,noRequests:true,serverUnchanged:true,retainedAfterRead:'https://neko-relay.czyt.eu.org'});
  await evaluate(`window.controller.dispose()`);
  await browser.navigate(`http://127.0.0.1:${server.httpServer.address().port}/remote-control-check`);
  await browser.command(["wait", "--fn", "window.ready === true"]);
  assert.equal(await evaluate(`document.querySelector('[name=enabled]').checked`), false);
  assert.equal(await evaluate(`document.querySelector('[data-remote-connection-options]').open`), false);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').checkVisibility() && document.querySelector('[data-remote-save]').checkVisibility()`), true);
  await browser.command(["click", "[data-remote-connection-options] > summary"]);
  await evaluate(`(async () => { const form = document.querySelector('[data-remote-config]'); form.elements.enabled.checked = true; form.elements.relay.value = 'https://relay.example'; form.elements.registrationKey.value = 'relay-test-key'; form.requestSubmit(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.actions[0].action`), "configure");
  assert.equal(await evaluate(`window.actions[0].registrationKey`), "relay-test-key");
  assert.equal(await evaluate(`document.querySelector('[name=registrationKey]').value`), "");
  assert.match(await evaluate(`document.querySelector('[data-remote-relay-rtt]').textContent`), /12 ms/);
  await evaluate(`(async () => { document.querySelector('[data-remote-invite]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`!!document.querySelector('.remote-qr')`), true);
  await evaluate(`(async () => { window.clock += 301000; await window.controller.refresh(); })()`);
  assert.equal(await evaluate(`!!document.querySelector('.remote-qr')`), false);
  await evaluate(`(async () => { window.remote.pending = [{id:'pending-1', name:'Phone <test>', fingerprint:'abc123', expiresInSeconds:120}]; await window.controller.refresh(); })()`);
  assert.equal(await evaluate(`document.querySelector("[data-remote-pending-section]").open`), false);
  assert.equal(await evaluate(`document.querySelector("[data-remote-pending-count]").textContent`), "1");
  await evaluate(`document.querySelector("[data-remote-pending-section]").open = true`);
  assert.equal(await evaluate(`document.querySelector('[data-pending] h5').textContent`), "Phone <test>");
  assert.equal(await evaluate(`document.querySelector('[data-pending] select').value`), "read_only");
  await evaluate(`document.querySelector('[data-pending]').requestSubmit()`);
  assert.equal(await evaluate(`window.actions.some(action => action.action === 'approve')`), false);
  await evaluate(`(async () => { const checkbox = document.querySelector('[name=session]'); checkbox.checked = true; checkbox.dispatchEvent(new Event('input', {bubbles:true})); const root = document.querySelector('[name="root:session-1"]'); root.value = '/home/user'; root.focus(); await window.controller.refresh(); })()`);
  assert.equal(await evaluate(`document.activeElement.value`), "/home/user");
  await evaluate(`(async () => { document.querySelector('[data-pending]').requestSubmit(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.deepEqual(await evaluate(`window.actions.find(action => action.action === 'approve').resources`), [{session: "session-1", root: "/home/user"}]);
  await evaluate(`document.querySelector("[data-remote-devices-section]").open = true`);
  assert.match(await evaluate(`document.querySelector('[data-device-rtt]').textContent`), /78 ms/);
  assert.equal(await evaluate(`document.querySelector('[data-device-management]').open`), false);
  assert.equal(await evaluate(`document.querySelector('[data-device-status]').checkVisibility() && document.querySelector('[data-device-rtt]').checkVisibility() && !document.querySelector('[data-device] select').checkVisibility()`), true);
  await browser.command(["click", "[data-device-management] > summary"]);
  await evaluate(`(async () => {
    window.permissionDraft = document.querySelector('[data-device] select'); window.permissionDraft.value = 'operate'; window.permissionDraft.focus();
    window.remote.devices[0].connections = 3; window.remote.devices[0].rttMs = 79;
    await window.controller.refresh();
  })()`);
  assert.equal(await evaluate(`document.querySelector('[data-device-management]').open && document.activeElement === window.permissionDraft && window.permissionDraft.value === 'operate'`), true);
  await evaluate(`(async () => { window.remote.devices[0].name = 'Phone renamed'; await window.controller.refresh(); })()`);
  assert.equal(await evaluate(`document.querySelector('[data-device-management]').open && document.activeElement === document.querySelector('[data-device] select') && document.activeElement.value === 'operate'`), true);
  await evaluate(`(async () => { window.remote.devices[0].name = 'Phone <test>'; await window.controller.refresh(); })()`);
  await browser.command(["click", "[data-device-management] > summary"]);
  await evaluate(`window.controller.refresh()`);
  assert.equal(await evaluate(`document.querySelector('[data-device-management]').open`), false);
  await browser.command(["click", "[data-device-management] > summary"]);
  await evaluate(`(async () => {
    window.remote.pending = [{id:'pending-draft', name:'Draft device', fingerprint:'draft-fingerprint', expiresInSeconds:120}];
    await window.controller.refresh();
    const relay = document.querySelector('[name=relay]'); relay.value = 'https://draft.example'; relay.dispatchEvent(new Event('input', {bubbles:true}));
    document.querySelector('[name=registrationKey]').value = 'unsaved-key';
    const checkbox = document.querySelector('[name=session]'); checkbox.checked = true; checkbox.dispatchEvent(new Event('input', {bubbles:true}));
    window.draftRoot = document.querySelector('[name="root:session-1"]'); window.draftRoot.value = '/unsaved/root'; window.draftRoot.focus();
    window.actionCount = window.actions.length;
    window.deferGet = true; window.refreshPromise = window.controller.refresh();
  })()`);
  assert.equal(await evaluate(`document.querySelector('[data-remote-working]').hidden`), false);
  assert.match(await evaluate(`document.querySelector('[data-remote-working]').textContent`), /Refreshing/);
  assert.equal(await evaluate(`document.querySelector('.remote-control').getAttribute('aria-busy')`), "true");
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('.remote-control button')).every(button => button.disabled)`), true);
  assert.equal(await evaluate(`document.activeElement === window.draftRoot && !window.draftRoot.disabled`), true);
  await evaluate(`document.querySelector('[data-revoke]').click(); document.querySelector('[data-permission]').click(); document.querySelector('[data-remote-config]').requestSubmit(); document.querySelector('[data-pending]').requestSubmit()`);
  assert.equal(await evaluate(`window.actions.length === window.actionCount`), true);
  await evaluate(`(async () => { window.deferGet = false; window.resolveGet(); await window.refreshPromise; })()`);
  assert.equal(await evaluate(`document.querySelector('[data-remote-working]').hidden`), true);
  assert.equal(await evaluate(`document.querySelector('[data-revoke]').disabled`), false);
  assert.equal(await evaluate(`document.querySelector('[data-remote-save]').disabled`), false);
  assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), "https://draft.example");
  assert.equal(await evaluate(`document.querySelector('[name=registrationKey]').value`), "unsaved-key");
  assert.equal(await evaluate(`document.activeElement === window.draftRoot && window.draftRoot.value === '/unsaved/root' && document.querySelector('[name=session]').checked`), true);
  for (const [name, draft] of [["relay", "https://typing-during-refresh.example"], ["registrationKey", "key-typed-during-refresh"]]) {
    await evaluate(`(() => {
      document.querySelector('[data-remote-connection-options]').open = true;
      window.focusedDraft = document.querySelector('[name=${name}]');
      window.previousDraft = window.focusedDraft.value;
      window.focusedDraft.focus();
      window.deferGet = true; window.refreshPromise = window.controller.refresh();
    })()`);
    assert.equal(await evaluate(`document.activeElement === window.focusedDraft && !window.focusedDraft.disabled`), true);
    assert.equal(await evaluate(`document.querySelector('[data-remote-save]').disabled && !document.querySelector('[data-remote-working]').hidden`), true);
    await browser.command(["fill", `[name=${name}]`, draft]);
    assert.equal(await evaluate(`document.activeElement === window.focusedDraft && window.focusedDraft.value === ${JSON.stringify(draft)}`), true);
    await evaluate(`(async () => { window.deferGet = false; window.resolveGet(); await window.refreshPromise; })()`);
    assert.equal(await evaluate(`document.activeElement === window.focusedDraft && !window.focusedDraft.disabled && window.focusedDraft.value === ${JSON.stringify(draft)}`), true);
    assert.equal(await evaluate(`document.querySelector('[data-remote-save]').disabled`), false);
    await evaluate(`window.focusedDraft.value = window.previousDraft; window.focusedDraft.dispatchEvent(new Event('input', {bubbles:true}))`);
  }
  await evaluate(`(async () => {
    document.querySelector('[name="root:session-1"]').focus(); window.locale = 'zh-CN'; await window.controller.refresh();
  })()`);
  assert.equal(await evaluate(`document.activeElement.name === 'root:session-1' && document.activeElement.value === '/unsaved/root' && document.querySelector('[name=session]').checked && document.querySelector('[data-remote-pending-section]').open`), true);
  await evaluate(`(async () => { window.locale = 'en'; await window.controller.refresh(); })()`);
  await evaluate(`(async () => { window.fail = true; await window.controller.refresh(); })()`);
  assert.match(await evaluate(`document.querySelector('[data-device-rtt]').textContent`), /Unavailable/);
  assert.equal(await evaluate(`document.querySelector('[name=registrationKey]').value`), "unsaved-key");
  assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), "https://draft.example");
  assert.equal(await evaluate(`document.querySelector('[name="root:session-1"]').value`), "/unsaved/root");
  assert.equal(await evaluate(`document.querySelector('[data-revoke]').disabled`), false);
  await evaluate(`(async () => { window.fail = false; await window.controller.refresh(); document.querySelector('[data-device] select').value = 'operate'; document.querySelector('[data-permission]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.actions.at(-1).permission`), "operate");
  assert.match(await evaluate(`window.confirmations.at(-1)`), /Phone <test>/);
  await evaluate(`(async () => { document.querySelector('[data-disconnect]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.remote.devices[0].online`), false);
  await evaluate(`(async () => { window.remote.enabled = false; await window.controller.refresh(); })()`);
  assert.equal(await evaluate(`!document.querySelector('[data-revoke]').disabled && !document.querySelector('[data-permission]').disabled && document.querySelector('[data-disconnect]').disabled && document.querySelector('[data-remote-invite]').disabled && document.querySelector('[data-pending] button[type=submit]').disabled`), true);
  await evaluate(`(async () => { window.remote.enabled = true; await window.controller.refresh(); })()`);
  for (const width of [360, 390]) {
    await browser.command(["set", "viewport", String(width), "844"]);
    for (const theme of ["light", "dark"]) {
      await evaluate(`Object.assign(document.querySelector('#fixture').dataset, { interfaceTone: '${theme}', interfaceStyle: '${theme === "light" ? "porcelain" : "default"}' })`);
      assert.equal(await evaluate(`document.documentElement.scrollWidth <= innerWidth`), true);
      assert.equal(await evaluate(`getComputedStyle(document.querySelector('[data-revoke]')).minHeight`), "48px");
      assert.equal(await evaluate(`getComputedStyle(document.querySelector('[data-device-management] > summary')).minHeight`), "48px");
    }
  }
  await evaluate(`(async () => { window.fail = true; document.querySelector('[data-revoke]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.remote.devices.length`), 1);
  assert.match(await evaluate(`document.querySelector('[data-remote-error]').textContent`), /Rejected/);
  await evaluate(`window.fail = false`);
  await evaluate(`(async () => { document.querySelector('[data-revoke]').focus(); document.querySelector('[data-revoke]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.remote.devices.length`), 0);
  assert.equal(await evaluate(`document.activeElement.hasAttribute('data-remote-refresh')`), true);
  await evaluate(`(async () => { window.remote.pending = [{id:'pending-reject',name:'Another device',fingerprint:'def456',expiresInSeconds:120}]; await window.controller.refresh(); document.querySelector('[data-reject]').click(); await new Promise(resolve => setTimeout(resolve, 0)); })()`);
  assert.equal(await evaluate(`window.actions.at(-1).action`), "reject");
  assert.equal(await evaluate(`window.remote.pending.length`), 0);
  await evaluate(`(async () => { window.locale = 'zh-CN'; await window.controller.refresh(); })()`);
  assert.match(await evaluate(`document.querySelector('[data-remote-state]').textContent`), /已连接/);
  const screenshotDirectory = process.env.REMOTE_CONTROL_SCREENSHOTS;
  if (screenshotDirectory) {
    for (const tone of ["light", "dark"]) {
      await evaluate(`Object.assign(document.querySelector('#fixture').dataset, { interfaceTone: '${tone}', interfaceStyle: '${tone === "light" ? "porcelain" : "default"}' }); document.querySelector('#fixture').scrollTop = 0`);
      await browser.command(["screenshot", `${screenshotDirectory}/remote-control-${tone}.png`, "--full"]);
    }
  }
  assert.equal(await evaluate(`document.querySelector('[data-remote-pending-section]').open && document.querySelector('[data-remote-devices-section]').open`), true);
  await evaluate(`(async () => {
    document.querySelector('[data-remote-pending-section]').open = false;
    document.querySelector('[data-remote-devices-section]').open = false;
    window.locale = 'en'; await window.controller.refresh();
  })()`);
  assert.equal(await evaluate(`document.querySelector('[data-remote-pending-section]').open || document.querySelector('[data-remote-devices-section]').open`), false);
  await evaluate(`document.querySelector('[data-remote-pending-section]').open = true; document.querySelector('[data-remote-devices-section]').open = true`);
  for (const method of ["get", "action", "invite"]) {
    await evaluate(`(async () => {
      window.authFailure = false;
      Object.assign(window.remote, {
        hostFingerprint: 'private-host-fingerprint', has_registration_key: true,
        operations: {inFlightUnknown:1, active:[{id:'operation-secret',deviceId:'sensitive-device',operation:'file_commit',outcomeUnknown:true}], recent:[]},
        devices:[{id:'sensitive-device',name:'Sensitive device',fingerprint:'private-device-fingerprint',permission:'operate',resources:[{id:'private-grant',name:'Sensitive resource',backend:'ssh',files:true}],online:true,connections:1,rttMs:42}],
        pending:[{id:'sensitive-pending',name:'Sensitive applicant',fingerprint:'private-applicant-fingerprint',expiresInSeconds:120}]
      });
      await window.controller.refresh();
      document.querySelector('[data-remote-invite]').click();
      await new Promise(resolve => setTimeout(resolve, 0));
      document.querySelector('[name=enabled]').checked = true; document.querySelector('[name=enabled]').dispatchEvent(new Event('input', {bubbles:true}));
      document.querySelector('[name=registrationKey]').value = 'private-key-draft';
      document.querySelector('[name=registrationKey]').dispatchEvent(new Event('input', {bubbles:true}));
      const checkbox = document.querySelector('[name=session]'); checkbox.checked = true; checkbox.dispatchEvent(new Event('input', {bubbles:true}));
      document.querySelector('[name="root:session-1"]').value = '/private-root-draft';
    })()`);
    assert.equal(await evaluate(`!!document.querySelector('.remote-qr') && !document.querySelector('[data-remote-host-fingerprint]').hidden && !document.querySelector('[data-remote-operation-warning]').hidden`), true);
    await browser.command(["click", "[data-device-management] > summary"]);
    await evaluate(`window.authFailure = true; window.actionCount = window.actions.length`);
    if (method === "get") await evaluate(`window.controller.refresh()`);
    else await evaluate(`document.querySelector('${method === "action" ? "[data-revoke]" : "[data-remote-invite]"}').click()`);
    await browser.command(["wait", "--fn", `document.querySelector('[data-remote-error]').textContent.includes('Owner verification failed') && document.querySelector('.remote-control').getAttribute('aria-busy') === 'false'`]);
    assert.equal(await evaluate(`document.querySelector('[data-remote-pending]').childElementCount + document.querySelector('[data-remote-devices]').childElementCount + document.querySelector('[data-remote-invitation]').childElementCount`), 0);
    assert.equal(await evaluate(`document.querySelector('[data-remote-host-fingerprint]').textContent + document.querySelector('[data-remote-operation-warning]').textContent + document.querySelector('[name=registrationKey]').value`), "");
    assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), 'https://draft.example');
    assert.equal(await evaluate(`document.querySelector('[data-remote-host-fingerprint]').hidden && document.querySelector('[data-remote-operation-warning]').hidden && document.querySelector('[data-remote-pairing]').hidden`), true);
    assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-remote-config] input, [data-remote-config] button')).every(element => !element.disabled) && document.querySelector('[data-remote-invite]').disabled`), true);
    assert.equal(await evaluate(`document.querySelector('[data-remote-refresh]').disabled`), false);
    assert.equal(await evaluate(`document.querySelector('[name=enabled]').checked`), true);
    await evaluate(`window.beforePoll = window.gets; window.clock += 10000; window.tick()`);
    assert.equal(await evaluate(`window.gets === window.beforePoll`), true);
    assert.doesNotMatch(await evaluate(`document.querySelector('.remote-control').textContent`), /Sensitive device|Sensitive resource|Sensitive applicant|private-.*fingerprint|NR1\./);
    await evaluate(`(async () => { window.authFailure = false; window.fail = true; await window.controller.refresh(); })()`);
    assert.equal(await evaluate(`document.querySelector('[data-remote-save]').disabled`), false);
    assert.equal(await evaluate(`document.querySelector('[data-remote-devices]').childElementCount`), 0);
    assert.equal(await evaluate(`window.actions.length`), await evaluate(`window.actionCount + ${method === "action" ? 1 : 0}`));
    await evaluate(`(async () => { window.fail = false; await window.controller.refresh(); })()`);
    assert.equal(await evaluate(`document.querySelector('[data-remote-save]').disabled`), false);
    assert.equal(await evaluate(`document.querySelector('[name=registrationKey]').value`), "");
    assert.equal(await evaluate(`document.querySelector('[name=relay]').value`), "https://draft.example");
    assert.equal(await evaluate(`document.querySelector('[name=session]').checked`), false);
    assert.equal(await evaluate(`document.querySelector('[name="root:session-1"]').value`), "");
  }
  await evaluate(`window.controller.dispose()`);
  console.log("Remote control browser checks passed: local activation without GET; editable first/failing configuration; health before configure, failed health prevents POST, disable without probe; successful-only polling and failure pause/recovery; late GET keeps per-field drafts; disabled state keeps offline administration; collapsed connection/device disclosures retain open state, focus, and permission drafts; deferred refresh disables actions and retains drafts; GET/action/invite 403 clears sensitive state and requires authorized recovery; default off, configuration, QR expiry, scoped approval, RTT freshness, permissions, disconnect/revoke, focus recovery, 360/390px light/dark layout.");
} finally { await browser.close(); await server.close(); }
