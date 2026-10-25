import type { PtyCallbacks, PtyTransport, ResttyFontInput, ResttyRuntimeEvent } from "restty";
import type { Instance } from "../gen/lazycat/webshell/v1/capability_pb";
import type { MessageKey } from "../i18n";
import { createPaneTerminal } from "../terminal-options";
import { escapeHtml } from "../utils";
import { createMicroVmSession, type SessionStatus } from "./session";
import { microVmTarget, createSession, removeSession, sessionSocket, sessionStore } from "./api";
import "./view.css";

type Options = {
  tr: (key: MessageKey, values?: Record<string, string | number>) => string;
  prepare: () => void;
  terminalOptions: () => { fonts: ResttyFontInput[]; fontSize: number };
};
export function createMicroVmTerminals(options: Options) {
  let dismiss: (() => void) | undefined;
  return { open(instance: Instance) {
    const target = microVmTarget(instance);
    if (!target || instance.terminalAvailable !== true || instance.status !== "running") return;
    dismiss?.(); options.prepare();
    const previousFocus = document.activeElement as HTMLElement | null;
    const dialog = document.createElement("dialog");
    dialog.className = "microvm-terminal-dialog";
    dialog.setAttribute("aria-labelledby", "microvm-terminal-title");
    dialog.innerHTML = `<header><strong id="microvm-terminal-title">${escapeHtml(target.name)}</strong><button type="button" data-action="dismiss">${escapeHtml(options.tr("microvm.detach"))}</button></header><div class="microvm-terminal-mount"></div><footer><span role="status"></span><div><button type="button" data-action="reconnect">${escapeHtml(options.tr("microvm.reconnect"))}</button><button type="button" data-action="new">${escapeHtml(options.tr("microvm.new"))}</button><button type="button" data-action="end">${escapeHtml(options.tr("microvm.end"))}</button></div></footer>`;
    const status = dialog.querySelector<HTMLElement>('[role="status"]')!;
    const reconnect = dialog.querySelector<HTMLButtonElement>('[data-action="reconnect"]')!;
    const newButton = dialog.querySelector<HTMLButtonElement>('[data-action="new"]')!;
    const endButton = dialog.querySelector<HTMLButtonElement>('[data-action="end"]')!;
    let callbacks: PtyCallbacks | undefined;
    let decoder = new TextDecoder();
    let closed = false, busy = false;
    let lastStatus: SessionStatus["state"] = "connecting";
    const store = sessionStore(target);
    function changed(event: SessionStatus) {
      if (closed) return;
      lastStatus = event.state;
      status.textContent = event.state === "ready" ? options.tr(event.gap ? "microvm.gap" : "microvm.ready")
        : event.state === "exited" ? options.tr(event.code === undefined ? "microvm.ended" : "microvm.exit", { code: event.code ?? 0 })
        : event.state === "error" ? options.tr("microvm.error", { message: event.message || "" })
        : options.tr(event.state === "connecting" ? "microvm.connecting" : "microvm.disconnected");
      reconnect.disabled = busy || event.state === "connecting" || event.state === "ready" || event.state === "exited";
      newButton.disabled = busy || event.state === "connecting";
      endButton.disabled = busy || event.state === "connecting";
      if (event.state === "ready") { if (event.gap) decoder = new TextDecoder(); callbacks?.onConnect?.(); }
      if (event.state === "exited" && event.code !== undefined) { const tail = decoder.decode(); if (tail) callbacks?.onData?.(tail); }
    }
    const session = createMicroVmSession({
      record: store.read(), save: store.save,
      create: record => createSession(target, record), remove: id => removeSession(target, id),
      socket: id => sessionSocket(target, id),
      output: bytes => callbacks?.onData?.(decoder.decode(bytes, { stream: true })), changed,
    });
    const transport: PtyTransport = {
      connect: connection => { callbacks = connection.callbacks; void session.connect(connection.cols || 80, connection.rows || 24); },
      disconnect: () => session.stop(), sendInput: data => session.send(data),
      resize: (cols, rows) => session.resize(cols, rows), isConnected: session.connected,
    };
    const terminal = createPaneTerminal({
      cols: 80, rows: 24, ...options.terminalOptions(), fontLigatures: false,
      fontHinting: true, fontHintTarget: "normal", scrollbackLimit: 5000,
      touchSelectionMode: "long-press", transport,
      beforeInput: ({ text }) => session.connected() ? text : "",
      contextMenuItems: () => [], searchClearButtonText: "", searchPlaceholder: "",
      onDomReady: () => {}, onGridSize: (cols, rows) => session.resize(cols, rows),
    });
    const close = () => {
      if (closed) return;
      closed = true; clearTimeout(startTimer); unsubscribe?.(); session.stop(); terminal.dispose(); dialog.close(); dialog.remove();
      if (previousFocus?.isConnected) previousFocus.focus();
      dismiss = undefined;
    };
    dismiss = close;
    dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
    dialog.addEventListener("keydown", event => { if (event.key === "Escape" && event.target instanceof Element && event.target.closest(".microvm-terminal-mount") && !event.shiftKey) event.preventDefault(); event.stopPropagation(); });
    dialog.addEventListener("click", async event => {
      const action = (event.target as Element).closest<HTMLElement>("[data-action]")?.dataset.action;
      if (action === "dismiss") { close(); return; }
      if (busy || !action) return;
      if (action === "reconnect") { session.reconnect(); terminal.focus(); return; }
      if (action === "new" && lastStatus !== "exited" && !window.confirm(options.tr("microvm.replaceConfirm"))) return;
      busy = true; newButton.disabled = endButton.disabled = reconnect.disabled = true;
      try {
        if (action === "end") await session.end();
        if (action === "new") { terminal.reset(); decoder = new TextDecoder(); await session.newSession(); terminal.focus(); }
      } catch (error) { status.textContent = options.tr("microvm.error", { message: String(error) }); }
      finally { busy = false; newButton.disabled = endButton.disabled = false; reconnect.disabled = session.connected(); }
    });
    document.body.append(dialog); dialog.showModal();
    terminal.open(dialog.querySelector<HTMLElement>(".microvm-terminal-mount")!);
    const pane = terminal.restty?.getPanes()[0];
    let started = false;
    const connect = () => { if (started || closed || !pane) return; started = true; clearTimeout(startTimer); pane.runtime.interaction.updateSize(true); pane.runtime.io.connectPty(""); terminal.focus(); };
    const startTimer = setTimeout(() => { if (!started && !closed) status.textContent = options.tr("microvm.error", { message: options.tr("microvm.initFailed") }); }, 15000);
    const unsubscribe = pane?.runtime.events.subscribe((event: ResttyRuntimeEvent) => {
      if (event.type === "state" && event.state === "ready") connect();
      if (event.type === "state" && event.state === "failed") status.textContent = options.tr("microvm.error", { message: options.tr("microvm.initFailed") });
    });
    if (pane?.runtime.lifecycle.state() === "ready") connect();
  } };
}
