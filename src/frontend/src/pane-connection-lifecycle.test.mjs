import assert from "node:assert/strict";
import test from "node:test";
import { canConnectPane } from "./pane-reconnect-policy.ts";
import { createPaneConnectionLifecycle } from "./pane-connection-lifecycle.ts";

function pane() {
  return {
    reconnectDelay: 1000,
    connectionState: "idle",
    hasConnected: true,
    sessionStatus: "running",
    sessionBackend: "webshell",
  };
}

function setup(online, overrides = {}) {
  const statuses = [];
  const timers = [];
  let connects = 0;
  const lifecycle = createPaneConnectionLifecycle({
    canConnect: () => true,
    autoRestartEnabled: () => false,
    isHerdr: () => false,
    isOnline: () => online,
    connect: () => { connects += 1; },
    setStatus: (_pane, message, tone) => statuses.push([message, tone]),
    tr: (key, values) => values ? `${key}:${values.seconds}` : key,
    random: () => 0,
    setTimer: (callback, delay) => {
      timers.push({ callback, delay });
      return timers.length;
    },
    clearTimer: () => undefined,
    ...overrides,
  });
  return { lifecycle, statuses, timers, connects: () => connects };
}

test("keeps bounded reconnect attempts active when navigator reports offline", () => {
  const target = pane();
  const subject = setup(false);
  subject.lifecycle.scheduleReconnect(target);
  assert.equal(target.connectionState, "offline");
  assert.equal(subject.timers[0].delay, 5000);
  subject.timers[0].callback();
  assert.equal(subject.connects(), 1);
});

test("projects connection lifecycle states and clears retry state on success", () => {
  const target = pane();
  const subject = setup(true);
  subject.lifecycle.beginConnection(target);
  assert.equal(target.connectionState, "reconnecting");
  subject.lifecycle.markReplaying(target);
  assert.equal(target.connectionState, "replaying");
  subject.lifecycle.markConnected(target);
  assert.equal(target.connectionState, "connected");
  assert.deepEqual(subject.statuses.at(-1), ["status.shellReady", "ok"]);
});

test("HC retries transport before ready without native auto-restart but stops on final exit", () => {
  for (const online of [true, false]) {
    for (const status of [undefined, "starting", "stopped", "exited"]) {
      const target = { ...pane(), sessionId: "hc-id", sessionBackend: "hc", sessionStatus: status,
        hasConnected: false, processExitObserved: false, exited: status === "exited" };
      const subject = setup(online, { canConnect: pane => canConnectPane(pane, false) });
      subject.lifecycle.scheduleReconnect(target);
      assert.equal(subject.timers.length, 1);
      assert.equal(subject.timers[0].delay, online ? 800 : 5000);
      subject.timers[0].callback();
      assert.equal(subject.connects(), 1);
      target.processExitObserved = true;
      subject.lifecycle.scheduleReconnect(target);
      assert.equal(subject.timers.length, 1);
    }
  }
  const native = { ...pane(), sessionStatus: "starting" };
  const subject = setup(true);
  subject.lifecycle.scheduleReconnect(native);
  assert.equal(subject.timers.length, 0);
  assert.equal(native.connectionState, "idle");
});
