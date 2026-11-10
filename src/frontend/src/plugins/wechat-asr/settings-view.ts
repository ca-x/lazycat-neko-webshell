import type { PluginDescriptor } from "../../gen/lazycat/webshell/v1/capability_pb";
import type { Translate } from "../host";
import type { WeChatAsrState } from "./controller";
import { escapeHtml } from "../../utils";

export function renderWeChatAsrSettings(plugin: PluginDescriptor, state: WeChatAsrState | undefined, disabled: boolean, tr: Translate): string {
  const busy = state?.busy ?? false;
  const ready = state?.ready ?? false;
  const status = busy
    ? tr(state?.operation === "download" ? "asr.modelDownloading" : state?.operation === "remove" ? "asr.modelRemoving" : "asr.modelChecking")
    : state?.loaded ? tr(ready ? "asr.modelReady" : "asr.modelMissing") : tr("asr.modelUnknown");
  const button = (action: string, label: string, danger = false) => `<button class="command-button${danger ? " danger" : ""}" type="button" data-wechat-asr-action="${action}" ${disabled || !plugin.enabled || busy ? "disabled" : ""}>${escapeHtml(label)}</button>`;
  return `<div class="asr-model-settings" aria-busy="${busy}">
    <div class="asr-model-row">
      <div class="asr-model-summary">
        <strong role="status" aria-live="polite">${escapeHtml(status)}</strong>
        <p class="settings-help">${escapeHtml(tr("asr.modelSource"))}</p>
      </div>
      <div class="asr-model-actions">
        ${state?.confirmRemove ? `${button("remove", tr("asr.confirmRemove"), true)}${button("cancel-remove", tr("action.cancel"))}`
          : `${ready ? button("confirm-remove", tr("asr.remove"), true) : button("download", tr(state?.error ? "asr.retryDownload" : "asr.download"))}${button("status", tr("asr.refresh"))}`}
      </div>
    </div>
    ${state?.error ? `<p class="settings-help" role="alert">${escapeHtml(state.error)}</p>` : ""}
    <p class="settings-help">${escapeHtml(tr("asr.localHelp"))}</p>
  </div>`;
}
