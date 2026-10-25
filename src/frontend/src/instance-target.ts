import type { Instance } from "./gen/lazycat/webshell/v1/capability_pb";

export function isMicroVm(instance: Instance | undefined): boolean {
  const kind = instance?.kind as unknown;
  return kind === 4 || kind === "INSTANCE_KIND_SMOLVM";
}

export function instanceTerminalAllowed(instance: Instance | undefined): boolean {
  return Boolean(instance && !isMicroVm(instance) && instance.terminalAvailable !== false);
}

export function microVmTerminalAvailable(instance: Instance): boolean {
  return isMicroVm(instance) && instance.terminalAvailable === true && instance.status === "running"
    && Boolean(instance.targetId) && ["community.lazycat.app.catflap", "community.lazycat.app.catflap.dev"].includes(instance.providerAppId);
}
