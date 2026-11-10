import type { PluginSettingsViewState } from "../plugin-views";
import { registeredAsrProviders } from "../plugins/asr/providers";
import { renderVoiceSettingsPanel } from "../plugins/ai-chat/settings/voice-panel-view";
import { renderAIConfigDialog } from "../plugins/ai-chat/settings/config-dialog-view";

export function renderVoiceSettingsView(state: PluginSettingsViewState): string {
  const dialog = state.aiAccess.dialog;
  const voiceState = {
    ...state.aiAccess,
    voiceProviderPlugins: registeredAsrProviders(state.plugins),
    disabled: state.pluginsLoading,
    tr: state.tr,
    dialog: dialog?.type === "voice" || dialog?.type === "voice-reply" ? dialog : undefined,
  };
  return `${renderVoiceSettingsPanel(voiceState)}${voiceState.dialog ? renderAIConfigDialog(voiceState) : ""}`;
}
