import type { Instance } from "./gen/lazycat/webshell/v1/capability_pb";

const warnings = new WeakMap<Instance[], string>();

export function recordDiscoveryWarnings(instances: Instance[], header: string | null): void {
  if (!header) return;
  try {
    const value: unknown = JSON.parse(header);
    if (Array.isArray(value) && value.every((item) => typeof item === "string")) {
      warnings.set(instances, value.join("; "));
    }
  } catch { /* An invalid diagnostic header must not hide valid instances. */ }
}

export function discoveryWarning(instances: Instance[]): string {
  return warnings.get(instances) || "";
}

export function instanceSourceDots(instance: Instance): string {
  return [...new Set(instance.sources || [])]
    .filter((source) => source === "lightos" || source === "catflap")
    .map((source) => `<span class="instance-source-dot instance-source-${source}" aria-hidden="true"></span>`)
    .join("");
}
