import assert from "node:assert/strict";
import test from "node:test";
import { createMicroVmSession, parseEvent, terminalSize } from "./session.ts";

class Socket extends EventTarget {
  readyState = 1; sent = [];
  send(value) { this.sent.push(value); }
  close() { if (this.readyState === 3) return; this.readyState = 3; this.dispatchEvent(new Event("close")); }
  message(data) { this.dispatchEvent(new MessageEvent("message", { data })); }
}
function harness(overrides = {}) {
  const saved = [], created = [], removed = [], sockets = [], states = [], outputs = [];
  const session = createMicroVmSession({
    save: value => saved.push(value && { ...value }), create: async value => { created.push(value); },
    remove: async id => { removed.push(id); }, socket: id => { const socket = new Socket(); sockets.push({ id, socket }); return socket; },
    output: bytes => outputs.push([...bytes]), changed: value => states.push(value), ...overrides,
  });
  return { session, saved, created, removed, sockets, states, outputs };
}
function ready(h, gap = false) { const { id, socket } = h.sockets.at(-1); socket.message(JSON.stringify({ type:"ready", session_id:id, cols:80, rows:24, replay:false, output_gap:gap })); }

test("saves identity before creating, applies startup size, and transmits raw bytes only after ready", async () => {
  const h = harness();
  await h.session.connect(91, 27);
  assert.deepEqual(h.saved[0], h.created[0]);
  assert.equal(h.created[0].cols, 91);
  assert.equal(h.created[0].rows, 27);
  assert.equal(h.session.send("early"), false);
  ready(h);
  assert.equal(h.session.send("中文\r\n"), true);
  const sent = h.sockets[0].socket.sent;
  assert.deepEqual(JSON.parse(sent[0]), { type:"resize", cols:91, rows:27 });
  assert.deepEqual(sent[1], new TextEncoder().encode("中文\r\n"));
  h.sockets[0].socket.message(new Uint8Array([0,255,13,10]).buffer);
  assert.deepEqual(h.outputs, [[0,255,13,10]]);
  h.session.stop();
});
test("disconnect reconnects the existing session and reports a gap without creating a shell", async () => {
  const h = harness(); await h.session.connect(80,24); ready(h);
  const id = h.sockets[0].id;
  h.sockets[0].socket.close(); h.session.reconnect(); ready(h, true);
  assert.equal(h.created.length, 1);
  assert.equal(h.sockets.at(-1).id, id);
  assert.deepEqual(h.states.at(-1), { state:"ready", gap:true });
  assert.equal(h.removed.length, 0); h.session.stop();
});
test("an ambiguous create response never causes automatic creation again", async () => {
  let attempts=0;
  const h = harness({ create: async () => { attempts++; throw new Error("timeout"); } });
  await h.session.connect(80,24);
  assert.equal(h.saved.length,1); assert.equal(h.sockets.length,0);
  h.session.reconnect(); assert.equal(h.sockets[0].id,h.saved[0].id);
  assert.equal(attempts,1); h.session.stop();
});
test("real exit codes follow output, survive reopening, and never trigger reconnect", async () => {
  const h = harness(); await h.session.connect(80,24); ready(h);
  h.sockets[0].socket.message(new Uint8Array([65]).buffer);
  h.sockets[0].socket.message(JSON.stringify({type:"process-exit",exit_code:7}));
  assert.deepEqual(h.outputs,[[65]]); assert.equal(h.saved.at(-1).exitCode,7);
  h.session.reconnect(); assert.equal(h.sockets.length,1);
  const restored=harness({record:h.saved.at(-1)}); await restored.session.connect(80,24);
  assert.equal(restored.created.length,0); assert.equal(restored.sockets.length,0);
  assert.deepEqual(restored.states.at(-1),{state:"exited",code:7}); h.session.stop(); restored.session.stop();
});
test("a failed explicit close retains identity and prevents a replacement shell", async () => {
  const h=harness({remove:async()=>{throw new Error("cleanup not confirmed");}});
  await h.session.connect(80,24);
  await assert.rejects(h.session.newSession(),/cleanup/);
  assert.equal(h.created.length,1); assert.equal(h.saved.length,1); h.session.stop();
});
test("expired sessions can be replaced only by explicit New; End does not claim cleanup", async () => {
  const h=harness({remove:async()=>{throw Object.assign(new Error("expired"),{status:403});}});
  await h.session.connect(80,24); const id=h.saved[0].id;
  await assert.rejects(h.session.end(),/expired/); assert.equal(h.created.length,1);
  await h.session.newSession(); assert.equal(h.created.length,2); assert.notEqual(h.saved.at(-1).id,id); h.session.stop();
});
test("control events reject mismatched identities, false replay and invented exit codes", () => {
  for (const value of [{type:"ready",session_id:"other",cols:80,rows:24,replay:false,output_gap:false},{type:"process-exit",exit_code:-1},{type:"process-exit",exit_code:256},{type:"process-exit",exit_code:"0"},{type:"output",data:""}]) assert.throws(()=>parseEvent(JSON.stringify(value),"expected"));
  assert.deepEqual(terminalSize(900,0),{cols:500,rows:24});
});
