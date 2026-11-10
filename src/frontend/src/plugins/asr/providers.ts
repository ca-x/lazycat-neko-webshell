import type { PluginDescriptor } from "../../gen/lazycat/webshell/v1/capability_pb";
import type { AiVoiceProviderProfile } from "../../types";

/** Plugins declare their ASR capability in the catalog; input does not infer it from an ID. */
export function registeredAsrProviders(plugins: readonly PluginDescriptor[]): PluginDescriptor[] {
  return plugins.filter(plugin => plugin.metadata.asrProvider === "true");
}
export function asrProfileAvailable(profile: AiVoiceProviderProfile | undefined, plugins: readonly PluginDescriptor[]): boolean {
  if (!profile || profile.provider !== "plugin") return true;
  return registeredAsrProviders(plugins).some(plugin => plugin.id === profile.pluginId && plugin.enabled);
}

// Disabling drains an in-flight model download/inference before releasing the engine.
export const ASR_PLUGIN_TIMEOUT_MS = 300_000;
export function pluginConfigurationTimeoutMs(plugin: PluginDescriptor): number {
  return plugin.metadata.asrProvider === "true" ? ASR_PLUGIN_TIMEOUT_MS : 10_000;
}
