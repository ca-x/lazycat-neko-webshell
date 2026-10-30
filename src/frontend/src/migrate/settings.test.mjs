import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

// Settings' existing module graph uses bundler-style extensionless imports.
const sourceRoot = new URL("../", import.meta.url).href;
const hook = registerHooks({
  resolve(specifier, context, nextResolve) {
    try { return nextResolve(specifier, context); }
    catch (error) {
      if (!specifier.startsWith(".") || !context.parentURL?.startsWith(sourceRoot)
        || !["ERR_MODULE_NOT_FOUND", "ERR_UNSUPPORTED_DIR_IMPORT"].includes(error.code)) throw error;
      for (const suffix of [".ts", "/index.ts"]) {
        try { return nextResolve(specifier + suffix, context); } catch { /* Try the directory form. */ }
      }
      throw error;
    }
  },
});
let migrateSettings;
try { ({ migrateSettings } = await import("./settings.ts")); }
finally { hook.deregister(); }

test("settings migration preserves HC and all existing backend choices; native stays the default", () => {
  for (const backend of ["webshell", "hc", "herdr", "zellij", "ssh"]) {
    assert.equal(migrateSettings({ defaultSessionBackend: backend }).defaultSessionBackend, backend);
  }
  assert.equal(migrateSettings({}).defaultSessionBackend, "webshell");
  assert.equal(migrateSettings({ defaultSessionBackend: "future" }).defaultSessionBackend, "webshell");
});

test("HC history migrates with a bounded default and preserves explicit zero", () => {
  assert.equal(migrateSettings({}).hcHistoryLines, 1000);
  for (const [value, expected] of [[0, 0], [10000, 10000], [12000, 10000], [-10, 0], [9.8, 9], ["bad", 1000]]) {
    assert.equal(migrateSettings({ hcHistoryLines: value }).hcHistoryLines, expected);
  }
});
