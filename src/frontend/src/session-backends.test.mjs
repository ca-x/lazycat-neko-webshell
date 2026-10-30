import assert from "node:assert/strict";
import test from "node:test";

import { preferredAvailableBackend } from "./session-backends.ts";

test("missing Herdr falls back for the current instance without changing the preference", () => {
  const state = {
    selector: "alpha@owner",
    backends: [{ id: "webshell", label: "WebShell native", available: true }],
  };
  const preferred = "herdr";
  assert.equal(preferredAvailableBackend(state, preferred), "webshell");
  assert.equal(preferred, "herdr");
  assert.equal(preferredAvailableBackend({ ...state, backends: [
    ...state.backends,
    { id: "herdr", label: "Herdr", available: true },
  ] }, preferred), "herdr");
});

test("HC is explicit and advertised, while native remains the default", async () => {
  const { normalizeSessionMode, selectableSessionBackends, sessionBackendLabel } = await import("./session-backends.ts");
  assert.equal(normalizeSessionMode("hc"), "hc");
  assert.equal(normalizeSessionMode(undefined), "webshell");
  assert.deepEqual(selectableSessionBackends(undefined).map(b => b.id), ["webshell"]);
  const state = { backends: [{ id: "hc", label: "hc", available: true }] };
  assert.equal(preferredAvailableBackend(state, "hc"), "hc");
  assert.equal(preferredAvailableBackend({ backends: [] }, "hc"), "hc");
  assert.equal(sessionBackendLabel("hc", "fallback", key => key), "backend.hc");
});

test("local splits include HC and keep backend inheritance available to orchestration", async () => {
  const { sessionBackendSupportsLocalSplit } = await import("./session-backends.ts");
  for (const backend of ["webshell", "hc", "ssh"]) assert.equal(sessionBackendSupportsLocalSplit(backend), true);
  for (const backend of ["herdr", "zellij"]) assert.equal(sessionBackendSupportsLocalSplit(backend), false);
});

test("unavailable explicit or default HC never resolves to native, including SSH targets", async () => {
  const { resolveNewSessionBackend, HcBackendUnavailableError } = await import("./session-backends.ts");
  for (const state of [undefined, { backends: [{ id: "webshell", available: true }, { id: "hc", available: false }] }]) {
    assert.throws(() => resolveNewSessionBackend(state, "webshell", "hc"), HcBackendUnavailableError);
    assert.throws(() => resolveNewSessionBackend(state, "hc"), HcBackendUnavailableError);
    assert.equal(preferredAvailableBackend(state, "hc", true), "hc");
    assert.equal(resolveNewSessionBackend(state, "hc", "webshell"), "webshell");
  }
  const available = { backends: [{ id: "hc", available: true }] };
  assert.equal(resolveNewSessionBackend(available, "webshell", "hc"), "hc");
  assert.equal(resolveNewSessionBackend(available, "hc"), "hc");
  for (const preferred of ["herdr", "zellij", "ssh"]) {
    assert.equal(resolveNewSessionBackend(undefined, preferred), "webshell");
    assert.equal(resolveNewSessionBackend(undefined, "webshell", preferred), "webshell");
  }
  assert.equal(preferredAvailableBackend(undefined, "herdr", true), "ssh");
});

test("settings retain an unavailable HC preference as a disabled explicit choice", async () => {
  const { renderSessionBackendSelectOptions } = await import("./session-backends.ts");
  const backends = [{ id: "webshell", label: "WebShell", available: true }];
  const html = renderSessionBackendSelectOptions(backends, key => key, "hc");
  assert.match(html, /<option value="hc" disabled>backend.hcUnavailable<\/option>/);
  assert.match(html, /<option value="webshell">/);
  assert.doesNotMatch(renderSessionBackendSelectOptions(backends, key => key, "herdr"), /value="hc"/);
});
