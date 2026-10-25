import { isMicroVm, instanceTerminalAllowed, microVmTerminalAvailable } from "./instance-target.ts";
import type { Instance } from "./gen/lazycat/webshell/v1/capability_pb";

export type InstanceGroupId = "lightos" | "smolvm" | "remote" | "ssh";

export type InstanceGroup = {
  id: InstanceGroupId;
  instances: Instance[];
};

export type InstanceRowPresentation = {
  kind: InstanceGroupId;
  metadata: string;
  running: boolean;
};

export function groupInstances(instances: Instance[]): InstanceGroup[] {
  const grouped = new Map<InstanceGroupId, Instance[]>([
    ["lightos", []],
    ["smolvm", []],
    ["remote", []],
    ["ssh", []],
  ]);
  for (const instance of instances) {
    grouped.get(instanceGroupId(instance))?.push(instance);
  }
  return (["lightos", "smolvm", "remote", "ssh"] as const)
    .map((id) => ({ id, instances: grouped.get(id) ?? [] }))
    .filter((group) => group.instances.length > 0);
}

export function instanceRowPresentation(instance: Instance): InstanceRowPresentation {
  const kind = instanceGroupId(instance);
  return {
    kind,
    metadata: kind === "smolvm" ? "MicroVM" : kind === "remote"
      ? String(instance.platform || "Remote client").trim()
      : kind === "ssh"
        ? "SSH"
        : String(instance.ownerDeployId || "LightOS").trim(),
    running: microVmTerminalAvailable(instance) || instanceTerminalAllowed(instance) && String(instance.status || "").trim() === "running" && Boolean(instanceSelectorValue(instance)),
  };
}

function instanceGroupId(instance: Instance): InstanceGroupId {
  if (isMicroVm(instance)) return "smolvm";
  const kind = instance.kind as unknown;
  if (kind === 2 || kind === "INSTANCE_KIND_REMOTE_CLIENT") return "remote";
  if (kind === 3 || kind === "INSTANCE_KIND_SSH") return "ssh";
  const selector = instanceSelectorValue(instance);
  if (selector.startsWith("client:")) return "remote";
  if (selector.startsWith("ssh:")) return "ssh";
  return "lightos";
}

function instanceSelectorValue(instance: Instance): string {
  const explicit = String(instance.selector || "").trim();
  if (explicit) return explicit;
  const name = String(instance.name || "").trim();
  const owner = String(instance.ownerDeployId || "").trim();
  return name && owner ? `${name}@${owner}` : "";
}
