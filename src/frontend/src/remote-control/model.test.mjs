import assert from "node:assert/strict";
import test from "node:test";
import { freshRtt, remoteSessionChoices, remoteStatus, unknownOperationCount } from "./model.ts";
import { invitationQr } from "./qr.ts";
import { remoteApi, RemoteApiError } from "./api.ts";

test("RTT expires at thirty seconds and never survives disconnect or missing samples", () => {
  assert.equal(freshRtt(42, true, 100, 30099), 42);
  assert.equal(freshRtt(42, true, 100, 30100), null);
  assert.equal(freshRtt(42, false, 100, 101), null);
  assert.equal(freshRtt(null, true, 100, 101), null);
  assert.equal(freshRtt(NaN, true, 100, 101), null);
  assert.equal(freshRtt(-1, true, 100, 101), null);
});

test("disabled configuration overrides a residual runtime online state", () => {
  assert.equal(remoteStatus({ enabled: false, status: "online" }), "disabled");
  assert.equal(remoteStatus({ enabled: true, status: "reconnecting" }), "reconnecting");
  assert.equal(remoteStatus({ enabled: true, status: "unknown" }), "offline");
});

test("resource candidates preserve complete selectors and deduplicate only session identity", () => {
  const pane = { sessionId: "one", selector: "target@owner", label: "Shell", sessionBackend: "webshell" };
  const choices = remoteSessionChoices([{ label: "Tab", panes: [pane, pane,
    { ...pane, sessionId: "two", selector: "ssh:profile" },
    { ...pane, sessionId: "closed", sessionStatus: "closed" },
    { ...pane, sessionId: undefined },
    { ...pane, sessionId: "client", selector: "client:machine" },
    { ...pane, sessionId: "vm", selector: "client:catflap:app:uuid" },
  ] }]);
  assert.deepEqual(choices.map(item => [item.id, item.selector]), [["one", "target@owner"], ["two", "ssh:profile"]]);
});

test("invitation QR is generated locally with a quiet zone and no secret-bearing URL", () => {
  const code = `NR1.${"A".repeat(43)}.${"B".repeat(43)}.${"C".repeat(43)}.1234567`;
  const svg = invitationQr(code);
  assert.match(svg, /viewBox="0 0 \d+ \d+"/);
  assert.match(svg, /M4,4h1v1h-1z/);
  assert.doesNotMatch(svg, /href=|NR1|<image|<script/);
});

test("management API sends only same-origin requests and propagates backend errors", async () => {
  const previousFetch = globalThis.fetch;
  const previousWindow = globalThis.window;
  const calls = [];
  globalThis.window = { location: { href: "https://webshell.example/app/" } };
  globalThis.fetch = async (url, options) => {
    calls.push({ url: String(url), options });
    return new Response(JSON.stringify({ error: "Owner verification failed" }), { status: 400 });
  };
  try {
    await assert.rejects(remoteApi.act({ action: "approve", pending: "p", permission: "read_only", resources: [{ session: "s", root: null }] }), /Owner verification failed/);
    assert.equal(calls[0].url, "https://webshell.example/app/api/remoted");
    assert.equal(calls[0].options.credentials, "same-origin");
    assert.equal(calls[0].options.cache, "no-store");
    assert.equal(calls[0].options.headers["x-neko-remoted-request"], "1");
    assert.equal(calls[0].options.headers["content-type"], "application/json");
    await assert.rejects(remoteApi.get(), /Owner verification failed/);
    assert.equal(calls[1].options.headers["x-neko-remoted-request"], "1");
    assert.deepEqual(JSON.parse(calls[0].options.body).resources, [{ session: "s", root: null }]);
  } finally { globalThis.fetch = previousFetch; globalThis.window = previousWindow; }
});

test("submitted operations with unknown results remain visible after device removal", () => {
  assert.equal(unknownOperationCount({}), 0);
  const operation = { id: "operation", deviceId: "removed-device", outcomeUnknown: true };
  assert.equal(unknownOperationCount({ devices: [], operations: { active: [operation], recent: [operation, { id: "done", outcomeUnknown: false }] } }), 1);
});

test("management errors retain HTTP status and authorization classification even without JSON", async () => {
  const previousFetch = globalThis.fetch;
  const previousWindow = globalThis.window;
  globalThis.window = { location: { href: "https://webshell.example/app/" } };
  try {
    for (const [status, body, authorizationLost, code] of [
      [403, JSON.stringify({ code: "AUTHORIZATION_FAILED", error: "Owner changed" }), true, "AUTHORIZATION_FAILED"],
      [401, "login required", true, undefined],
      [403, "forbidden", true, undefined],
      [400, JSON.stringify({ error: "Invalid resource" }), false, undefined],
      [503, "unavailable", false, undefined],
    ]) {
      globalThis.fetch = async () => new Response(body, { status });
      await assert.rejects(remoteApi.get(), error => {
        assert.ok(error instanceof RemoteApiError);
        assert.equal(error.status, status);
        assert.equal(error.code, code);
        assert.equal(error.authorizationLost, authorizationLost);
        return true;
      });
    }
  } finally { globalThis.fetch = previousFetch; globalThis.window = previousWindow; }
});
