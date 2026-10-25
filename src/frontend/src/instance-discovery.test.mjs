import assert from "node:assert/strict";
import test from "node:test";
import { instanceSourceDots, discoveryWarning, recordDiscoveryWarnings } from "./instance-discovery.ts";

test("sources use only fixed colored dots without text or hover explanations", () => {
  const html = instanceSourceDots({ sources: ["lightos", "catflap", "catflap", '<script>'] });
  assert.equal((html.match(/aria-hidden="true"/g) || []).length, 2);
  assert.ok(html.includes("instance-source-lightos"));
  assert.ok(html.includes("instance-source-catflap"));
  assert.doesNotMatch(html, /title=|script|>[^<]+</);
  assert.equal(instanceSourceDots({}), "");
});

test("partial discovery warnings belong to their own response, not a global user cache", () => {
  const alice = [], bob = [];
  recordDiscoveryWarnings(alice, JSON.stringify(["LightOS unavailable", "Catflap timeout"]));
  assert.equal(discoveryWarning(alice), "LightOS unavailable; Catflap timeout");
  assert.equal(discoveryWarning(bob), "");
  for (const value of [null, "invalid", '{}', '[1]']) {
    recordDiscoveryWarnings(bob, value);
    assert.equal(discoveryWarning(bob), "");
  }
});
