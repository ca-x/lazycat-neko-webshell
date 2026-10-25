import assert from "node:assert/strict";
import test from "node:test";
import { instanceSourceDots } from "./instance-discovery.ts";

test("sources use only fixed colored dots without text or hover explanations", () => {
  const html = instanceSourceDots({ sources: ["lightos", "catflap", "catflap", '<script>'] });
  assert.equal((html.match(/aria-hidden="true"/g) || []).length, 2);
  assert.ok(html.includes("instance-source-lightos"));
  assert.ok(html.includes("instance-source-catflap"));
  assert.doesNotMatch(html, /title=|script|>[^<]+</);
  assert.equal(instanceSourceDots({}), "");
});
