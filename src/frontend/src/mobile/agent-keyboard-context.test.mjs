import assert from "node:assert/strict";
import test from "node:test";

import {
  createMobileAgentKeyboardContext,
  mobileAgentKeyboardContext,
} from "./agent-keyboard-context.ts";

function record(app, overrides = {}) {
  return { id: "agent-1", app, state: "working", ...overrides };
}

function nativePane(overrides = {}) {
  return {
    id: "pane-1",
    sessionId: "session-1",
    selector: "dev@owner-1",
    sessionBackend: "webshell",
    connectionState: "connected",
    exited: false,
    closing: false,
    replaying: false,
    title: "shell",
    programStatus: [record("codex")],
    ...overrides,
  };
}

function herdrPane(overrides = {}) {
  return {
    pane_id: "remote-pane-1",
    workspace_id: "workspace-1",
    tab_id: "tab-1",
    focused: true,
    agent: "codex",
    agent_status: "working",
    tokens: {},
    ...overrides,
  };
}

function herdrAgent(overrides = {}) {
  return {
    terminal_id: "terminal-1",
    pane_id: "remote-pane-1",
    workspace_id: "workspace-1",
    tab_id: "tab-1",
    focused: true,
    agent: "codex",
    agent_status: "working",
    revision: 1,
    launch_pending: false,
    interactive_ready: true,
    state_change_seq: 1,
    tokens: {},
    ...overrides,
  };
}

function herdrState(overrides = {}) {
  return {
    selector: "dev@owner-1",
    available: true,
    resources_complete: true,
    focused_pane_id: "remote-pane-1",
    panes: [herdrPane()],
    agents: [herdrAgent()],
    ...overrides,
  };
}

function herdrClient(overrides = {}) {
  return nativePane({ sessionBackend: "herdr", programStatus: [record("claude")], ...overrides });
}

function assertAgent(context, agent) {
  assert.ok(context, `expected a ${agent} keyboard context`);
  assert.equal(context.agent, agent);
  assert.equal(typeof context.key, "string");
  assert.ok(context.key.length > 0);
  return context;
}

function freeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const item of Object.values(value)) freeze(item);
    Object.freeze(value);
  }
  return value;
}

function factory(initial = {}) {
  const state = { enabled: true, pane: nativePane(), herdr: undefined, ...initial };
  let changes = 0;
  const context = createMobileAgentKeyboardContext({
    enabled: () => state.enabled,
    pane: () => state.pane,
    herdr: () => state.herdr,
    changed: () => { changes += 1; },
  });
  return { state, context, changes: () => changes };
}

test("strict opt-out returns no context without inspecting pane or Herdr identity", () => {
  const unreadable = new Proxy({}, {
    get(_target, property) { assert.fail(`disabled resolver read ${String(property)}`); },
  });
  for (const enabled of [false, undefined]) {
    assert.equal(mobileAgentKeyboardContext(enabled, unreadable, unreadable), undefined);
    const { context, changes } = factory({ enabled, pane: unreadable, herdr: unreadable });
    assert.equal(context.current(), undefined);
    context.refresh();
    context.refresh();
    assert.equal(changes(), 0);
  }
});

test("a missing active pane cannot borrow identity from a populated Herdr cache", () => {
  assert.equal(mobileAgentKeyboardContext(true, undefined, herdrState()), undefined);
});

for (const app of ["codex", "claude", "pi", "opencode"]) {
  test(`native ${app} uses structured app identity despite conflicting titles`, () => {
    const pane = nativePane({
      title: app === "codex" ? "Claude Code" : "Codex CLI",
      programStatus: [record(app, { title: "OpenCode", msg: "Pi is running" })],
    });
    assertAgent(mobileAgentKeyboardContext(true, pane, herdrState()), app);
  });
}

for (const [app, expected] of [
  ["Claude-Code", "claude"], ["CODEX", "codex"], ["OpenCode", "opencode"], ["omp", "pi"],
]) {
  test(`structured app alias ${app} resolves to ${expected}`, () => {
    assertAgent(mobileAgentKeyboardContext(true, nativePane({ programStatus: [record(app)] })), expected);
  });
}

for (const [label, records] of [
  ["missing status", undefined],
  ["empty replacement", []],
  ["title-only status", [record(undefined, { title: "Codex CLI", msg: "Claude Code" })]],
  ["blank app", [record("")]],
  ["whitespace app", [record("  ")]],
  ["unknown app", [record("my-agent")]],
  ["unsupported known Agent", [record("gemini")]],
  ["an app merely containing a known Agent name", [record("my-codex-wrapper")]],
]) {
  test(`native ${label} stays static despite Agent titles and another cache`, () => {
    const pane = nativePane({ title: "Codex CLI / Claude Code / Pi / OpenCode", programStatus: records });
    assert.equal(mobileAgentKeyboardContext(true, pane, herdrState()), undefined);
  });
}

test("native records may agree on one normalized Agent without choosing a record by activity", () => {
  const pane = nativePane({ programStatus: [
    record("claude", { id: "a", state: "idle" }),
    record("Claude-Code", { id: "b", state: "blocked" }),
    record(undefined, { id: "c", title: "Codex" }),
  ] });
  assertAgent(mobileAgentKeyboardContext(true, pane), "claude");
});

for (const apps of [["codex", "claude"], ["pi", "opencode"], ["codex", "unknown"], ["codex", "gemini"]]) {
  test(`native conflicting identities ${apps.join(" / ")} cannot select the first Agent`, () => {
    const programStatus = apps.map((app, index) => record(app, { id: `agent-${index}`, state: index ? "blocked" : "working" }));
    assert.equal(mobileAgentKeyboardContext(true, nativePane({ programStatus })), undefined);
    assert.equal(mobileAgentKeyboardContext(true, nativePane({ programStatus: [...programStatus].reverse() })), undefined);
  });
}

const inactivePanes = [
  ...["idle", "connecting", "replaying", "reconnecting", "offline", "fatal"].map((connectionState) => [connectionState, { connectionState }]),
  ["replaying flag", { replaying: true }],
  ["exited", { exited: true }],
  ["closing", { closing: true }],
];

for (const [label, overrides] of inactivePanes) {
  for (const backend of ["native", "Herdr"]) {
    test(`${backend} ${label} immediately has no Agent context`, () => {
      const pane = backend === "native" ? nativePane(overrides) : herdrClient(overrides);
      assert.equal(mobileAgentKeyboardContext(true, pane, herdrState()), undefined);
    });
  }
}

test("an empty native replacement invalidates the factory and does not retain a previous Agent", () => {
  const { state, context, changes } = factory();
  const first = assertAgent(context.current(), "codex");
  context.refresh();
  assert.equal(changes(), 1);
  state.pane.programStatus = [];
  assert.equal(context.current(), undefined);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 2);
  state.pane.programStatus = undefined;
  context.refresh();
  assert.equal(changes(), 2);
  state.pane.programStatus = [record("pi", { id: "agent-2" })];
  assert.notEqual(assertAgent(context.current(), "pi").key, first.key);
  context.refresh();
  assert.equal(changes(), 3);
});

for (const [label, overrides] of [
  ["pane", { id: "pane-2" }],
  ["session", { sessionId: "session-2" }],
  ["full selector", { selector: "dev@owner-2" }],
  ["backend", { sessionBackend: "ssh" }],
  ["Agent", { programStatus: [record("pi")] }],
  ["Agent status record", { programStatus: [record("codex", { id: "agent-2" })] }],
]) {
  test(`native context key changes with ${label} identity`, () => {
    const first = assertAgent(mobileAgentKeyboardContext(true, nativePane()), "codex");
    const next = assertAgent(mobileAgentKeyboardContext(true, nativePane(overrides)), label === "Agent" ? "pi" : "codex");
    assert.notEqual(next.key, first.key);
  });
}

test("native context key ignores status order and presentation updates without mutating records", () => {
  const pane = freeze(nativePane({ programStatus: [record("codex", { id: "a" }), record("codex", { id: "b" })] }));
  const first = assertAgent(mobileAgentKeyboardContext(true, pane), "codex");
  const updated = freeze({
    ...pane,
    title: "Claude Code",
    programStatus: pane.programStatus.toReversed().map((item) => ({
      ...item, state: "blocked", kind: "question", progress: 75, title: "Pi", msg: "OpenCode",
    })),
  });
  assert.deepEqual(mobileAgentKeyboardContext(true, updated), first);
  assert.deepEqual(pane.programStatus.map((item) => item.id), ["a", "b"]);
});

for (const app of ["codex", "claude", "pi", "opencode"]) {
  test(`Herdr ${app} resolves from the focused pane or its associated structured Agent`, () => {
    const pane = herdrClient();
    for (const [panes, agents] of [
      [[herdrPane({ agent: app })], []],
      [[herdrPane({ agent: undefined })], [herdrAgent({ agent: app })]],
      [[herdrPane({ agent: app })], [herdrAgent({ agent: app })]],
    ]) {
      assertAgent(mobileAgentKeyboardContext(true, pane, herdrState({ panes, agents })), app);
    }
  });
}

test("a Herdr programKind uses Herdr identity even before the backend label agrees", () => {
  assertAgent(mobileAgentKeyboardContext(true, nativePane({ programKind: "herdr", programStatus: [record("pi")] }), herdrState()), "codex");
  assert.equal(mobileAgentKeyboardContext(true, nativePane({ programKind: "herdr" }), undefined), undefined);
});

for (const [label, overrides] of [
  ["unavailable cache", { available: false }],
  ["incomplete resources", { resources_complete: false }],
  ["missing completeness marker", { resources_complete: undefined }],
  ["different selector", { selector: "other@owner-1" }],
  ["same name but different selector owner", { selector: "dev@owner-2" }],
  ["missing focused pane ID", { focused_pane_id: undefined }],
  ["empty focused pane ID", { focused_pane_id: "" }],
  ["unmatched focused pane ID", { focused_pane_id: "remote-pane-missing" }],
  ["missing focused pane resource", { panes: [] }],
  ["focused resource without focused flag", { panes: [herdrPane({ focused: false })] }],
  ["another focused pane", { panes: [herdrPane({ focused: false }), herdrPane({ pane_id: "remote-pane-2" })] }],
  ["failed restore", { panes: [herdrPane({ restore_error: "restore failed" })] }],
  ["pending Agent launch", { agents: [herdrAgent({ launch_pending: true })] }],
]) {
  test(`Herdr ${label} rejects stale identities and native fallback`, () => {
    assert.equal(mobileAgentKeyboardContext(true, herdrClient(), herdrState(overrides)), undefined);
  });
}

test("Herdr requires a supplied cache even when native programStatus names a known Agent", () => {
  assert.equal(mobileAgentKeyboardContext(true, herdrClient(), undefined), undefined);
});

test("Herdr titles, display names and Agent names are never structured identities", () => {
  const titles = {
    title: "Codex CLI", terminal_title: "Claude Code", terminal_title_stripped: "Pi", display_agent: "OpenCode",
  };
  const state = herdrState({
    panes: [herdrPane({ ...titles, agent: undefined })],
    agents: [herdrAgent({ ...titles, name: "codex", agent: undefined })],
  });
  assert.equal(mobileAgentKeyboardContext(true, herdrClient({ title: "Codex CLI" }), state), undefined);
});

test("Herdr only considers Agents associated with the actual focused pane", () => {
  const state = herdrState({
    panes: [herdrPane({ agent: undefined }), herdrPane({ pane_id: "remote-pane-2", focused: false, agent: "claude" })],
    agents: [
      herdrAgent({ pane_id: "remote-pane-2", agent: "claude", focused: true, launch_pending: true }),
      herdrAgent({ agent: "pi", focused: false }),
    ],
  });
  assertAgent(mobileAgentKeyboardContext(true, herdrClient(), state), "pi");
  assert.equal(mobileAgentKeyboardContext(true, herdrClient(), { ...state, agents: state.agents.slice(0, 1) }), undefined);
});

test("Herdr matching pane and Agent aliases may agree without making metadata an identity", () => {
  const state = herdrState({
    panes: [herdrPane({ agent: "Claude-Code", title: "Pi" })],
    agents: [herdrAgent({ agent: "claude", display_agent: "Codex", name: "OpenCode" })],
  });
  assertAgent(mobileAgentKeyboardContext(true, herdrClient(), state), "claude");
});

for (const [label, paneAgent, agents] of [
  ["unknown pane identity", "custom-agent", []],
  ["unsupported Agent identity", undefined, ["gemini"]],
  ["pane and Agent conflict", "codex", ["claude"]],
  ["multiple associated Agents conflict", undefined, ["pi", "opencode"]],
  ["known and unknown identities conflict", "codex", ["unknown"]],
]) {
  test(`Herdr ${label} cannot choose one known Agent`, () => {
    const state = herdrState({
      panes: [herdrPane({ agent: paneAgent })],
      agents: agents.map((agent, index) => herdrAgent({ agent, terminal_id: `terminal-${index}` })),
    });
    assert.equal(mobileAgentKeyboardContext(true, herdrClient(), state), undefined);
    assert.equal(mobileAgentKeyboardContext(true, herdrClient(), { ...state, agents: [...state.agents].reverse() }), undefined);
  });
}

test("a pending associated launch invalidates an otherwise agreeing Herdr pane and Agent", () => {
  const state = herdrState({ agents: [herdrAgent(), herdrAgent({ terminal_id: "terminal-2", launch_pending: true })] });
  assert.equal(mobileAgentKeyboardContext(true, herdrClient(), state), undefined);
});

test("Herdr context key changes when the remote focused pane changes with the same Agent", () => {
  const pane = herdrClient();
  const state = herdrState({
    panes: [herdrPane(), herdrPane({ pane_id: "remote-pane-2", focused: false })],
    agents: [herdrAgent(), herdrAgent({ pane_id: "remote-pane-2", terminal_id: "terminal-2", focused: false })],
  });
  const first = assertAgent(mobileAgentKeyboardContext(true, pane, state), "codex");
  const next = assertAgent(mobileAgentKeyboardContext(true, pane, {
    ...state,
    focused_pane_id: "remote-pane-2",
    panes: state.panes.map((item) => ({ ...item, focused: item.pane_id === "remote-pane-2" })),
  }), "codex");
  assert.notEqual(next.key, first.key);
});

test("Herdr Agent session replacement changes its context key and refreshes the keyboard", () => {
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState() });
  const first = assertAgent(context.current(), "codex");
  context.refresh();
  state.herdr = { ...state.herdr, agents: [herdrAgent({ terminal_id: "replacement-terminal" })] };
  const next = assertAgent(context.current(), "codex");
  context.refresh();
  context.refresh();
  assert.deepEqual(
    { sameKey: next.key === first.key, notifications: changes() },
    { sameKey: false, notifications: 2 },
    "a new Agent session must invalidate gesture callbacks captured for the previous session",
  );
});

test("factory current is synchronous and pure and does not consume a pending refresh", () => {
  const pane = freeze(nativePane());
  const cached = freeze(herdrState());
  const { state, context, changes } = factory({ pane, herdr: cached });
  assert.equal(changes(), 0);
  const first = assertAgent(context.current(), "codex");
  assert.deepEqual(context.current(), first);
  assert.equal(changes(), 0);
  context.refresh();
  assert.equal(changes(), 1);
  state.pane = freeze(nativePane({ programStatus: [record("pi")] }));
  assertAgent(context.current(), "pi");
  assertAgent(context.current(), "pi");
  assert.equal(changes(), 1);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 2);
});

test("factory refresh notifies once per identity or validity transition and stays inert while disabled", () => {
  const { state, context, changes } = factory({ enabled: false });
  context.refresh();
  assert.equal(changes(), 0);
  state.enabled = true;
  context.refresh();
  context.refresh();
  assert.equal(changes(), 1);
  state.pane = { ...state.pane, title: "Pi", programStatus: [record("codex", { state: "done", progress: 100 })] };
  context.refresh();
  assert.equal(changes(), 1);
  state.pane = { ...state.pane, sessionId: "session-2" };
  context.refresh();
  assert.equal(changes(), 2);
  state.enabled = false;
  context.refresh();
  context.refresh();
  assert.equal(context.current(), undefined);
  assert.equal(changes(), 3);
  state.pane.programStatus = [record("pi")];
  state.herdr = herdrState();
  context.refresh();
  assert.equal(context.current(), undefined);
  assert.equal(changes(), 3);
  state.enabled = true;
  assertAgent(context.current(), "pi");
  context.refresh();
  assert.equal(changes(), 4);
  state.pane = undefined;
  context.refresh();
  context.refresh();
  assert.equal(changes(), 5);
});

test("factory refresh invalidates each unsafe pane state once and can recover from it", () => {
  for (const makePane of [nativePane, herdrClient]) {
    for (const [label, overrides] of inactivePanes) {
      const { state, context, changes } = factory({ pane: makePane(), herdr: herdrState() });
      context.refresh();
      state.pane = makePane(overrides);
      assert.equal(context.current(), undefined, label);
      context.refresh();
      context.refresh();
      assert.equal(changes(), 2, label);
      state.pane = makePane();
      assertAgent(context.current(), "codex");
      context.refresh();
      context.refresh();
      assert.equal(changes(), 3, label);
    }
  }
});

test("factory uses replacement Herdr snapshots and never retains an empty or invalid old cache", () => {
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState() });
  context.refresh();
  state.herdr = herdrState({ panes: [], agents: [] });
  assert.equal(context.current(), undefined);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 2);
  state.herdr = herdrState({ available: false });
  context.refresh();
  state.herdr = herdrState({ resources_complete: false });
  context.refresh();
  state.herdr = herdrState({ selector: "dev@owner-2" });
  context.refresh();
  assert.equal(changes(), 2);
  state.herdr = herdrState();
  assertAgent(context.current(), "codex");
  context.refresh();
  context.refresh();
  assert.equal(changes(), 3);
});

test("factory current keeps a frozen Herdr snapshot pure without publishing changes", () => {
  const pane = freeze(herdrClient());
  const cached = freeze(herdrState());
  const { state, context, changes } = factory({ pane, herdr: cached });
  const first = assertAgent(context.current(), "codex");
  assert.deepEqual(context.current(), first);
  assert.equal(changes(), 0);
  context.refresh();
  state.herdr = freeze(herdrState({
    panes: [herdrPane({ agent: "pi" })],
    agents: [herdrAgent({ agent: "pi" })],
  }));
  assertAgent(context.current(), "pi");
  assert.equal(changes(), 1);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 2);
});

test("factory ignores Herdr presentation, activity and unrelated background updates", () => {
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState() });
  const first = assertAgent(context.current(), "codex");
  context.refresh();
  state.herdr = freeze(herdrState({
    panes: [
      herdrPane({ pane_id: "background-pane", agent: "pi", focused: false, restore_error: "failed" }),
      herdrPane({ title: "Claude Code", agent_status: "blocked", tokens: { input: 100 } }),
    ],
    agents: [
      herdrAgent({ pane_id: "background-pane", terminal_id: "background-terminal", agent: "pi", launch_pending: true }),
      herdrAgent({ title: "OpenCode", agent_status: "blocked", revision: 2, state_change_seq: 2 }),
    ],
  }));
  assert.deepEqual(context.current(), first);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 1);
});

test("factory invalidates unknown and ambiguous replacements instead of caching the previous Agent", () => {
  for (const app of ["unknown", "claude"]) {
    const { state, context, changes } = factory();
    context.refresh();
    state.pane = nativePane({ programStatus: [record("codex"), record(app, { id: "agent-2" })] });
    assert.equal(context.current(), undefined);
    context.refresh();
    context.refresh();
    assert.equal(changes(), 2);
  }
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState() });
  context.refresh();
  state.herdr = herdrState({ agents: [herdrAgent({ agent: "claude" })] });
  assert.equal(context.current(), undefined);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 2);
});


test("Herdr associated Agent ordering does not change identity or refresh the keyboard", () => {
  const agents = [herdrAgent(), herdrAgent({ terminal_id: "terminal-2" })];
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState({ agents }) });
  const first = assertAgent(context.current(), "codex");
  context.refresh();
  state.herdr = herdrState({ agents: agents.toReversed().map((item) => ({
    ...item, revision: 9, agent_status: "done", state_change_seq: 50,
  })) });
  assert.deepEqual(context.current(), first);
  context.refresh();
  context.refresh();
  assert.equal(changes(), 1);
});

test("lost Herdr metadata validity falls back while PTY stays connected and recovers from an identical snapshot", () => {
  const { state, context, changes } = factory({ pane: herdrClient(), herdr: herdrState() });
  context.refresh();
  const before = context.current();
  context.setHerdrSnapshotValid(false);
  assert.equal(state.pane.connectionState, "connected");
  assert.equal(state.herdr.available, true);
  assert.equal(context.current(), undefined);
  assert.equal(changes(), 2);
  context.refresh();
  context.setHerdrSnapshotValid(false);
  assert.equal(changes(), 2);
  context.setHerdrSnapshotValid(true);
  assert.deepEqual(context.current(), before);
  assert.equal(changes(), 3, "an identical successful snapshot must repaint after invalidation");
});

test("Herdr metadata failure never suppresses native status or enables an opted-out keyboard", () => {
  const { state, context, changes } = factory();
  context.refresh();
  const native = context.current();
  context.setHerdrSnapshotValid(false);
  assert.deepEqual(context.current(), native);
  assert.equal(changes(), 1);
  state.enabled = false;
  context.refresh();
  context.setHerdrSnapshotValid(true);
  assert.equal(context.current(), undefined);
  assert.equal(changes(), 2);
});
