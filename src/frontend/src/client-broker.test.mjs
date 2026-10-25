import assert from "node:assert/strict";
import test from "node:test";
import { isPhysicalClientSelector, launchSelector } from "./client-broker.ts";
import { preservePhysicalSelection, updateWorkspaceLocation, clearWorkspaceLocation } from "./workspace-selection.ts";

const id = "pc-0123456789abcdef0123456789abcdef";
const app = "community.lazycat.app.catflap";
const selector = `client:catflap:${app}:${id}`;

test("Catflap launch preserves an allowlisted source in the canonical selector", () => {
  const params = new URLSearchParams({ name: `client:${id}`, broker: "catflap", catflap_app_id: app });
  assert.equal(launchSelector(params), selector);
  assert.equal(launchSelector(new URLSearchParams({ name: selector })), selector);
  assert.ok(isPhysicalClientSelector(selector));
  assert.ok(isPhysicalClientSelector(`client:${id}`));
  assert.ok(preservePhysicalSelection(selector, true));
  assert.ok(preservePhysicalSelection("client:invalid-broker:unsupported", true));
  assert.equal(preservePhysicalSelection(selector, false), false);
});

test("unknown brokers, mismatched apps and malformed registrations never select LightOS", () => {
  for (const params of [
    { broker: "unknown" },
    { name: `client:${id}`, broker: "unknown" },
    { name: `client:${id}`, broker: "catflap", catflap_app_id: "evil.example" },
    { name: selector, broker: "catflap", catflap_app_id: `${app}.dev` },
    { name: "client:../escape", broker: "catflap", catflap_app_id: app },
    { name: `client:${id}`, catflap_app_id: app },
  ]) {
    const result = launchSelector(new URLSearchParams(params));
    assert.equal(result, "client:invalid-broker:unsupported");
    assert.equal(isPhysicalClientSelector(result), false);
  }
});

test("changing or clearing selection removes stale launch hints without losing the canonical source", () => {
  const previous = globalThis.window;
  const calls = [];
  globalThis.window = {
    location: { href: `https://neko.example/?name=client:old&broker=catflap&catflap_app_id=${app}` },
    history: { state: {}, replaceState(state, unused, url) { calls.push({ state, url }); } },
  };
  try {
    updateWorkspaceLocation(selector, { tabId: "tab-a" });
    const selected = calls.pop();
    assert.equal(selected.url.searchParams.get("name"), selector);
    assert.equal(selected.url.searchParams.get("tab"), "tab-a");
    assert.equal(selected.url.searchParams.has("broker"), false);
    assert.equal(selected.url.searchParams.has("catflap_app_id"), false);
    clearWorkspaceLocation();
    assert.equal(calls.pop().url.search, "");
  } finally {
    if (previous === undefined) delete globalThis.window;
    else globalThis.window = previous;
  }
});
