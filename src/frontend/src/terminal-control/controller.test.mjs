import assert from "node:assert/strict";
import test from "node:test";
import { createTerminalControlController } from "./controller.ts";

test("HC remains writable in single-controller mode without unsupported control requests", async () => {
  const sent = [], statuses = [];
  const pane = { id: "hc-pane", sessionId: "hc-session", sessionBackend: "hc", cols: 80, rows: 24,
    socket: { readyState: 1, send: message => sent.push(message) },
    mount: { classList: { toggle() {} } } };
  const overlayRoot = { hidden: false, innerHTML: "", addEventListener() {} };
  const controller = createTerminalControlController({
    settings: () => ({ terminalSingleControllerMode: true, terminalBlurObservers: true }),
    overlayRoot, activePane: () => pane, panes: () => [pane], tr: key => key,
    onStatus: (...args) => statuses.push(args), onRenderIcons() {}, onTakeControlResize() {},
  });
  assert.deepEqual(await controller.prepareAttach(pane), { cols: 80, rows: 24 });
  assert.equal(controller.canWrite(pane), true);
  assert.equal(await controller.takeControl(pane), false);
  assert.equal(await controller.releaseControl(pane), false);
  controller.render();
  assert.equal(overlayRoot.hidden, true);
  assert.deepEqual(sent, []);
  assert.deepEqual(statuses, []);
  const native = { ...pane, sessionBackend: "webshell" };
  assert.deepEqual(await controller.prepareAttach(native), { cols: 80, rows: 24, controlMode: "single" });
  assert.equal(controller.canWrite(native, { report: false }), false);
});
