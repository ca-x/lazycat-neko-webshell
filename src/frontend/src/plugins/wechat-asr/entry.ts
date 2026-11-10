import "./styles.css";
import type { PluginDescriptor } from "../../gen/lazycat/webshell/v1/capability_pb";
import type { PluginSettingsViewState } from "../../plugin-views";
import type { createWeChatAsrController } from "./controller";
import { settingsDisabled, type PluginEntry } from "../host";
import { renderWeChatAsrSettings } from "./settings-view";
export type EntryResources = { controller: ReturnType<typeof createWeChatAsrController> };
export function createEntry(resources: EntryResources): PluginEntry {
  return { id: "wechat-asr", configured: plugin => resources.controller.configured(plugin.enabled) };
}
export function renderSettings(plugin: PluginDescriptor, state: PluginSettingsViewState): string {
  return renderWeChatAsrSettings(plugin, state.wechatAsr, settingsDisabled(plugin, state), state.tr);
}
export function isSettingsOpen(): boolean { return false; }
