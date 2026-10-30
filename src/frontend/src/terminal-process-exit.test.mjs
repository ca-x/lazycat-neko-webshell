import assert from "node:assert/strict";
import test from "node:test";
import { observeTerminalProcessExit } from "./terminal-process-exit.ts";
import { parseTerminalServerMessage } from "./terminal-protocol.ts";
import { canConnectPane, shouldConnectRestoredPane } from "./pane-reconnect-policy.ts";

const tr = (key, values = {}) => ({
  "status.processExited": `Process exited: ${values.code}`,
  "status.processTerminated": "Process terminated (exit code unavailable)",
  "status.processTerminatedSignal": `Process terminated by signal: ${values.signal}`,
})[key];

test("HC null-code signal termination is final without inventing a successful exit", () => {
  const codes = [];
  const pane = { sessionBackend: "hc", sessionId: "same-session", sessionStatus: "running",
    transport: { notifyExit: code => codes.push(code) } };
  const event = parseTerminalServerMessage('{"type":"process-exit","exit_code":null,"signal":15,"reason":"signaled"}');
  const message = observeTerminalProcessExit(pane, event, tr);
  assert.equal(message, "Process terminated by signal: 15 (signaled)");
  assert.deepEqual(codes, [-1]);
  assert.equal(event.exit_code, null);
  assert.equal(pane.processExitObserved, true);
  assert.equal(pane.exited, true);
  assert.equal(pane.sessionStatus, "exited");
  assert.equal(canConnectPane(pane, true), false);
  assert.equal(shouldConnectRestoredPane({ ...pane, sessionStatus: "running" }, true), false);
});

test("termination without a code remains unknown; real exit codes including zero are retained", () => {
  const cases = [
    [{ exit_code: null }, "Process terminated (exit code unavailable)", -1],
    [{ reason: "finished" }, "Process terminated (exit code unavailable) (finished)", -1],
    [{ exit_code: null, signal: "SIGTERM" }, "Process terminated by signal: SIGTERM", -1],
    [{ exit_code: 0 }, "Process exited: 0", 0],
    [{ exit_code: 7 }, "Process exited: 7", 7],
    [{ exit_code: null, message: "Target terminated" }, "Target terminated", -1],
  ];
  for (const [event, expectedMessage, expectedCode] of cases) {
    const codes = [];
    const pane = { transport: { notifyExit: code => codes.push(code) } };
    assert.equal(observeTerminalProcessExit(pane, { type: "process-exit", ...event }, tr), expectedMessage);
    assert.deepEqual(codes, [expectedCode]);
    assert.equal(pane.processExitObserved, true);
  }
});
