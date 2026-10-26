import { herdrAgentIconKey } from "../herdr-agent-icons.ts";
import type { HerdrBridgeState, TerminalPane } from "../types.ts";
import type { MobileKeyboardAgent } from "./agent-keymap.ts";

type Pane = Pick<TerminalPane, "id" | "sessionId" | "selector" | "sessionBackend" | "programKind"
  | "connectionState" | "exited" | "closing" | "replaying" | "programStatus">;
type HerdrState = Pick<HerdrBridgeState, "selector" | "available" | "resources_complete"
  | "focused_pane_id" | "panes" | "agents">;

export type MobileAgentKeyboardContext = { agent: MobileKeyboardAgent; key: string };

function agentIdentity(names: Array<string | undefined>): MobileKeyboardAgent | undefined {
  const identities = new Set(names.filter((name) => name?.trim()).map((name) => {
    const icon = herdrAgentIconKey(name);
    return icon === "claudecode" ? "claude" : icon ?? `unknown:${name}`;
  }));
  if (identities.size !== 1) return undefined;
  const [identity] = identities;
  return identity === "claude" || identity === "codex" || identity === "opencode" || identity === "pi"
    ? identity : undefined;
}

export function mobileAgentKeyboardContext(
  enabled: boolean,
  pane: Pane | undefined,
  herdr: HerdrState | undefined,
): MobileAgentKeyboardContext | undefined {
  if (!enabled || !pane || pane.connectionState !== "connected" || pane.exited || pane.closing || pane.replaying) return undefined;
  let agent: MobileKeyboardAgent | undefined;
  let identity: unknown;
  if (pane.sessionBackend === "herdr" || pane.programKind === "herdr") {
    if (!herdr?.available || !herdr.resources_complete || herdr.selector !== pane.selector || !herdr.focused_pane_id) return undefined;
    const focused = herdr.panes.find((item) => item.pane_id === herdr.focused_pane_id && item.focused);
    if (!focused || focused.restore_error) return undefined;
    const agents = herdr.agents.filter((item) => item.pane_id === focused.pane_id);
    if (agents.some((item) => item.launch_pending)) return undefined;
    agent = agentIdentity([focused.agent, ...agents.map((item) => item.agent)]);
    identity = [focused.pane_id, agents.map((item) => `${item.terminal_id}:${item.agent ?? ""}`).sort()];
  } else {
    const records = pane.programStatus ?? [];
    agent = agentIdentity(records.map((record) => record.app));
    identity = records.map((record) => `${record.id}:${record.app ?? ""}`).sort();
  }
  return agent ? { agent, key: JSON.stringify([pane.id, pane.sessionId, pane.selector, pane.sessionBackend, agent, identity]) } : undefined;
}

export function createMobileAgentKeyboardContext(options: {
  enabled: () => boolean;
  pane: () => Pane | undefined;
  herdr: () => HerdrState | undefined;
  changed: () => void;
}) {
  let key: string | undefined;
  let herdrSnapshotValid = true;
  const current = () => mobileAgentKeyboardContext(options.enabled(), options.pane(), herdrSnapshotValid ? options.herdr() : undefined);
  function refresh() {
    const next = current()?.key;
    if (next === key) return;
    key = next;
    options.changed();
  }
  return {
    current,
    refresh,
    setHerdrSnapshotValid(valid: boolean) {
      herdrSnapshotValid = valid;
      refresh();
    },
  };
}
