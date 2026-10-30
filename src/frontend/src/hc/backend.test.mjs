import assert from "node:assert/strict";
import test from "node:test";
import { webshellTerminalSocketUrl } from "../webshell-backend.ts";
import { createTerminalReplayController } from "../terminal-replay-controller.ts";
import { paneReplayAfter } from "../pane-input-buffer.ts";

test("HC attach keeps identity, requests full paint and cannot request native restart/control", () => {
  globalThis.window = { location: new URL("https://example.test/app/") };
  const url = webshellTerminalSocketUrl({
    selector: "box@owner", sessionId: "same-session", paneId: "pane-1", sessionBackend: "hc",
    cols: 80, rows: 24, restart: true, after: 999, outputLimit: 4096,
    terminalReplyAuthority: "client", controlMode: "single",
  });
  assert.equal(url.pathname, "/app/ws/terminal");
  assert.equal(url.searchParams.get("backend"), "hc");
  assert.equal(url.searchParams.get("session_id"), "same-session");
  assert.equal(url.searchParams.get("terminal_reply_authority"), "server");
  assert.equal(url.searchParams.get("restart"), "false");
  assert.equal(url.searchParams.get("after"), "0");
  assert.equal(url.searchParams.has("control_mode"), false);
  assert.equal(paneReplayAfter({ sessionBackend: "hc", lastOutputSequence: 999 }), 0);
});

test("each HC attach resets before its first paint, including a slow multi-frame repaint", async () => {
  const events = [];
  const pane = { sessionBackend: "hc", lastOutputSequence: 999,
    term: { reset: () => events.push("reset") }, decoder: new TextDecoder(), closing: false };
  const controller = createTerminalReplayController({
    byteBudget: 1,
    writeBytes: (p, b) => { assert.equal(p.replaying, true); events.push([...b]); },
    updateSequence: () => {}, onUnlocked: () => events.push("unlocked"),
    requestFrame: () => 1, cancelFrame: () => {}, setTimer: () => 1, clearTimer: () => {},
    nextFrame: async () => {}, now: () => 0,
  });
  const oldDecoder = pane.decoder;
  for (let i = 0; i < 2; i++) {
    controller.begin(pane);
    assert.equal(events.filter(e => e === "reset").length, i);
    controller.validate(pane);
    controller.push(pane, new Uint8Array([65, 66]));
    await controller.finish(pane);
  }
  assert.deepEqual(events, ["reset", [65], [66], "unlocked", "reset", [65], [66], "unlocked"]);
  assert.notEqual(pane.decoder, oldDecoder);
  assert.equal(pane.lastOutputSequence, 0);
});

test("only HC URLs request bounded history lines; native URLs stay unchanged", () => {
  globalThis.window = { location: new URL("https://example.test/") };
  const options = { selector: "box", sessionId: "id", paneId: "pane", cols: 80, rows: 24,
    restart: false, after: 0, outputLimit: 4096, terminalReplyAuthority: "client" };
  for (const [value, expected] of [[undefined, "1000"], [0, "0"], [10000, "10000"], [99999, "10000"]]) {
    const url = webshellTerminalSocketUrl({ ...options, sessionBackend: "hc", hcHistoryLines: value });
    assert.equal(url.searchParams.get("hc_history_lines"), expected);
  }
  for (const backend of ["webshell", "herdr", "zellij", "ssh"]) {
    assert.equal(webshellTerminalSocketUrl({ ...options, sessionBackend: backend, hcHistoryLines: 42 }).searchParams.has("hc_history_lines"), false);
  }
});
