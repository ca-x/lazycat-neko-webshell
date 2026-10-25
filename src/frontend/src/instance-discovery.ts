import type { Instance } from "./gen/lazycat/webshell/v1/capability_pb";

export function instanceSourceDots(instance: Instance): string {
  return [...new Set(instance.sources || [])]
    .filter((source) => source === "lightos" || source === "catflap")
    .map((source) => `<span class="instance-source-dot instance-source-${source}" aria-hidden="true"></span>`)
    .join("");
}
