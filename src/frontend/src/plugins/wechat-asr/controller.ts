import type { PluginJsonInvoker } from "../invoke-json";
import { errorMessage } from "../../utils";

export type WeChatAsrState = { ready: boolean; busy: boolean; operation?: "status" | "download" | "remove"; loaded: boolean; error: string; confirmRemove: boolean };
export function createWeChatAsrController(options: { root: HTMLElement; invoke: PluginJsonInvoker; refresh: () => void }) {
  let enabled = false;
  let generation = 0;
  let state: WeChatAsrState = { ready: false, busy: false, loaded: false, error: "", confirmRemove: false };
  async function request(operation: "status" | "download" | "remove") {
    if (!enabled || state.busy) return;
    const current = ++generation;
    state = { ...state, busy: true, operation, error: "", confirmRemove: false };
    options.refresh();
    try {
      const reply = await options.invoke("wechat-asr", "", operation, {});
      if (generation !== current || !enabled) return;
      state = { ...state, ready: reply.ready === true, loaded: true };
    } catch (error) {
      if (generation !== current || !enabled) return;
      state = { ...state, error: errorMessage(error) };
    } finally {
      if (generation === current && enabled) { state = { ...state, busy: false, operation: undefined }; options.refresh(); }
    }
  }
  options.root.addEventListener("click", event => {
    const button = event.target instanceof Element ? event.target.closest<HTMLButtonElement>("[data-wechat-asr-action]") : null;
    if (!button || button.disabled || !enabled || state.busy) return;
    const action = button.dataset.wechatAsrAction;
    if (action === "confirm-remove") { state = { ...state, confirmRemove: true }; options.refresh(); }
    else if (action === "cancel-remove") { state = { ...state, confirmRemove: false }; options.refresh(); }
    else if (action === "status" || action === "download" || (action === "remove" && state.confirmRemove)) void request(action);
  });
  return {
    state: (): WeChatAsrState => ({ ...state }),
    configured(next: boolean) {
      if (enabled === next) return;
      enabled = next; generation++;
      state = { ...state, ready: false, busy: false, operation: undefined, error: "", loaded: false, confirmRemove: false };
      if (enabled) void request("status");
    },
  };
}
