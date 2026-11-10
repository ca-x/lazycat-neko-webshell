import assert from "node:assert/strict";
import { createServer } from "vite";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createBrowserDriver, uniqueBrowserSession } from "../browser-driver.mjs";
import { createSettingsUiPreview } from "../settings-ui/preview-server.mjs";

export async function runAsrProviderScenario() {
  let ready = false, attempts = 0;
  let recordingServer;
  const calls = [];
  const server = await createSettingsUiPreview(0, { invokePlugin: async body => {
    calls.push(body);
    assert.equal(body.pluginId, "wechat-asr");
    assert.equal(body.sessionId ?? "", "");
    if (body.operation === "download") {
      attempts++;
      if (attempts === 1) throw Error("fixture download failure");
      await new Promise(resolve => setTimeout(resolve, 500));
      ready = true;
    }
    if (body.operation === "remove") ready = false;
    return { payload: Buffer.from(JSON.stringify({ ready })).toString("base64"), contentType: "application/json" };
  } });
  const browser = createBrowserDriver({ session: uniqueBrowserSession("asr-providers"), headed: false, launchArgs: "--use-fake-device-for-media-stream,--use-fake-ui-for-media-stream,--autoplay-policy=no-user-gesture-required" });
  const evaluate = source => browser.evaluate(source);
  const click = async selector => {
    if (selector.startsWith("[data-settings-tab=")) {
      if (await evaluate('matchMedia("(max-width: 760px)").matches && !document.querySelector(".settings-navigation").open')) await browser.command(["click", ".settings-navigation-trigger"]);
    }
    return browser.command(["click", selector]);
  };
  const wait = source => browser.waitFor(source);
  const artifacts = fileURLToPath(new URL("../artifacts/asr-providers/", import.meta.url));
  await mkdir(artifacts, { recursive: true });
  try {
    await browser.open(`http://127.0.0.1:${server.httpServer.address().port}`);
    await browser.command(["set", "viewport", "1280", "900"]);
    await wait('document.querySelector("#settingsButton")');
    await click("#settingsButton"); await click("#openSettingsItem");
    await wait('!document.querySelector("#settingsPage").hidden');
    await click("[data-settings-tab=plugins]");
    await wait('document.querySelector("[data-plugin-settings=wechat-asr]")');
    await click("[data-plugin-settings=wechat-asr] summary");
    assert.equal(await evaluate('document.querySelector("[data-wechat-asr-action=download]").disabled'), true);
    await click("[data-plugin-toggle=wechat-asr]");
    await wait('!document.querySelector("[data-wechat-asr-action=download]").disabled');
    assert.equal(await evaluate('document.querySelectorAll("[data-plugin-settings=wechat-asr] input:not([type=checkbox]),[data-plugin-settings=wechat-asr] textarea").length'), 0);
    await browser.screenshot(artifacts + "missing-desktop.png");
    await click("[data-wechat-asr-action=download]");
    await wait('document.querySelector("[data-plugin-settings=wechat-asr] [role=alert]")');
    assert.equal(await evaluate('document.querySelector("[data-wechat-asr-action=download]").textContent.includes("Retry")'), true);
    await click("[data-wechat-asr-action=download]");
    await wait('document.querySelector("[data-wechat-asr-action=confirm-remove]")');
    await browser.screenshot(artifacts + "ready-desktop.png");
    await click("[data-wechat-asr-action=confirm-remove]");
    assert.equal(calls.filter(call => call.operation === "remove").length, 0);
    await click("[data-wechat-asr-action=cancel-remove]");
    assert.equal(ready, true);
    await click("[data-plugin-toggle=ai-chat]");
    await wait('!document.querySelector("[data-plugin-toggle=ai-chat]").checked');
    await click("[data-settings-tab=voice]");
    assert.equal(await evaluate('document.querySelector("[data-ai-voice-new=true]").disabled'), false);
    assert.equal(await evaluate('document.querySelector("[data-ai-settings-tab=voice]")'), null);
    await click("[data-ai-voice-new=true]");
    await wait('document.querySelector("[data-ai-dialog-field=voiceProvider]")');
    await browser.command(["select", "[data-ai-dialog-field=voiceProvider]", "plugin:wechat-asr"]);
    assert.equal(await evaluate('[...document.querySelectorAll("[data-voice-api-field]")].every(field=>!field.checkVisibility())'), true);
    assert.equal(await evaluate('document.querySelector("[data-ai-dialog-field=voiceFormat]").value'), "wav");
    await browser.screenshot(artifacts + "provider-desktop.png");
    await click("[data-ai-config-save]");
    await wait('!document.querySelector("[data-ai-config-modal]")');
    await click("[data-ai-config-open=voice][data-ai-voice-profile-id]");
    await wait('document.querySelector("[data-ai-dialog-field=voiceProvider]")?.value==="plugin:wechat-asr"');
    await browser.press("Escape");
    await click("[data-ai-voice-enabled]");
    await wait('document.querySelector("[data-ai-voice-enabled]").checked');
    await browser.command(["set", "viewport", "390", "844"]);
    await browser.screenshot(artifacts + "settings-mobile.png");
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'), true);
    await click("[data-settings-tab=plugins]");
    await click("[data-wechat-asr-action=confirm-remove]");
    await click("[data-wechat-asr-action=remove]");
    await wait('document.querySelector("[data-wechat-asr-action=download]")');
    assert.equal(ready, false);
    assert.equal(calls.filter(call => call.operation === "download").length, 2);
    assert.equal(calls.filter(call => call.operation === "remove").length, 1);
    recordingServer = await createServer({ configFile: false, root: fileURLToPath(new URL("../../", import.meta.url)), server: { host: "127.0.0.1", port: 0 }, logLevel: "error" });
    await recordingServer.listen();
    await browser.navigate(`http://127.0.0.1:${recordingServer.httpServer.address().port}/tests-auto/asr-providers/fixture.html`);
    await browser.command(["set", "viewport", "1280", "900"]);
    await wait('window.qa?.ready');
    const voice = "[data-terminal-input-action=voice]";
    const primary = ".terminal-input-action-button";
    assert.equal(await evaluate('document.querySelector("[data-terminal-desktop-voice]")'), null);
    await click(primary);
    await browser.command(["focus", voice]); await browser.press("Enter");
    await wait('document.querySelector(".terminal-input-actions-surface").dataset.state==="recording" && window.qa.streams.length===1');
    assert.equal(await evaluate('document.querySelector(".terminal-input-action-menu").hidden'), true);
    assert.equal(await evaluate('document.activeElement.classList.contains("terminal-input-action-button")'), true);
    await browser.screenshot(artifacts + "recording-desktop.png");
    await evaluate('window.qa.switchProfile()');
    await browser.command(["focus", primary]); await browser.press("Enter");
    await wait('window.qa.uploads.length===1 && window.qa.stopped()');
    assert.deepEqual(await evaluate('window.qa.uploads.map(({profileId,provider,pluginId,mime})=>({profileId,provider,pluginId,mime}))'), [{ profileId: "local", provider: "plugin", pluginId: "wechat-asr", mime: "audio/wav" }]);
    assert.deepEqual(await evaluate('window.qa.sent'), ["echo hello"]);
    await evaluate('window.qa.resetProfile()');
    await click(primary); await click(voice); await wait('window.qa.streams.length===2');
    await evaluate('window.qa.disable()');
    await wait('window.qa.stopped()');
    assert.equal(await evaluate('window.qa.uploads.length'), 1);
    await click(primary);
    assert.equal(await evaluate('document.querySelector("[data-terminal-input-action=voice]").disabled'), true);
    await evaluate('window.qa.enable()');
    await click(voice); await wait('window.qa.streams.length===3');
    await browser.press("Escape"); await wait('window.qa.stopped()');
    assert.equal(await evaluate('window.qa.uploads.length'), 1);
    await evaluate('window.qa.mobile(true)');
    await browser.command(["set", "viewport", "390", "844"]);
    assert.equal(await evaluate('document.querySelector("[data-terminal-desktop-voice]")'), null);
    await click(".terminal-input-action-button");
    await wait('!document.querySelector(".terminal-input-action-menu").hidden');
    assert.equal(await evaluate('document.querySelector("[data-terminal-input-action=voice]").disabled'), false);
    await browser.screenshot(artifacts + "voice-mobile.png");
    const point = await evaluate('(()=>{const r=document.querySelector("[data-terminal-input-action=voice]").getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()');
    await browser.dispatchTouch([
      { type: "touchStart", delayMs: 600, touchPoints: [{ ...point, id: 1 }] },
      { type: "touchEnd", touchPoints: [] },
    ]);
    await wait('window.qa.uploads.length===2 && window.qa.stopped()');
    assert.deepEqual(await evaluate('window.qa.sent'), ["echo hello", "echo hello"]);
    return { status: "passed", scenario: "asr-providers", downloads: attempts, artifacts };
  } catch (error) {
    console.error(JSON.stringify({ calls, ready, attempts, ui: await evaluate('document.querySelector("[data-plugin-settings=wechat-asr]")?.textContent') }));
    await browser.screenshot(artifacts + "failure.png");
    throw error;
  } finally { await browser.close(); await server.close(); await recordingServer?.close(); }
}
