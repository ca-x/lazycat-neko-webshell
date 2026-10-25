import assert from "node:assert/strict";
import test from "node:test";
import {
  instanceSelector,
  isRunningInstance,
  forgetRememberedWorkspace,
  lastTabStorageKey,
  rememberSelector,
  shouldClearWorkspaceSelection,
} from "./workspace-selection.ts";

test("forgets the selected workspace and its last tab without clearing another selector", () => {
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  rememberSelector("client:pc", storage);
  storage.setItem(lastTabStorageKey("client:pc"), "tab-1");
  storage.setItem(lastTabStorageKey("client:other"), "tab-other");

  assert.equal(forgetRememberedWorkspace("client:other", storage), false);
  assert.equal(values.get(lastTabStorageKey("client:other")), "tab-other");
  assert.equal(forgetRememberedWorkspace("client:pc", storage), true);
  assert.equal(values.has("lazycat-neko-webshell.lastSelector"), false);
  assert.equal(values.has(lastTabStorageKey("client:pc")), false);
});

test("clears selection only when the selected workspace is empty and no tab remains", () => {
  assert.equal(shouldClearWorkspaceSelection("client:pc", "client:pc", 0, false), true);
  assert.equal(
    shouldClearWorkspaceSelection(
      "debian-bak@cloud.lazycat.lightos.entry",
      "debian-bak@cloud.lazycat.lightos.entry",
      0,
      false,
    ),
    false,
  );
  assert.equal(shouldClearWorkspaceSelection("client:pc", "client:pc", 1, false), false);
  assert.equal(shouldClearWorkspaceSelection("client:pc", "client:other", 0, false), false);
  assert.equal(shouldClearWorkspaceSelection("client:pc", "client:pc", 0, true), false);
});

test("MicroVM identities cannot become selectors or auto-selected terminal targets", () => {
  const vm = { kind: 4, status: "running", targetId: "12345678-1234-4234-8234-123456789abc", selector: "forged@owner", terminalAvailable: true };
  assert.equal(instanceSelector(vm), "");
  assert.equal(isRunningInstance(vm), false);
});
