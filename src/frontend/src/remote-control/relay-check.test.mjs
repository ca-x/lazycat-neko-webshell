import assert from 'node:assert/strict';
import test from 'node:test';
import { checkRelayAddress, relayHealthUrl, RelayCheckError } from './relay-check.ts';

class Socket {
  static instances = [];
  constructor(url, protocol) { this.url = url; this.protocol = protocol; Socket.instances.push(this); }
  close() { this.closed = true; }
  message(data) { this.onmessage?.({data}); }
}
const failure = reason => error => error instanceof RelayCheckError && error.reason === reason;

test('health URL accepts only HTTPS/WSS origins and never business paths', async () => {
  assert.equal(relayHealthUrl(' https://relay.example:8443/ '), 'wss://relay.example:8443/healthz');
  assert.equal(relayHealthUrl('wss://relay.example'), 'wss://relay.example/healthz');
  for (const address of ['', 'http://relay.example', 'ws://relay.example', 'https://u:p@relay.example', 'https://@relay.example', 'https://relay.example/host', 'https://relay.example/client', 'https://relay.example/.', 'https://relay.example?', 'https://relay.example#', 'https://relay.example\\']) {
    await assert.rejects(checkRelayAddress(address, {WebSocket: Socket}), failure('address'));
  }
  assert.equal(Socket.instances.length, 0);
});

test('opening the socket alone is insufficient; valid JSON health response closes it', async () => {
  const result = checkRelayAddress('https://relay.example', {WebSocket: Socket});
  const socket = Socket.instances.at(-1);
  assert.equal(socket.url, 'wss://relay.example/healthz');
  assert.equal(socket.protocol, 'neko.remoted.v1');
  let done = false;
  result.then(() => done = true);
  await Promise.resolve();
  assert.equal(done, false);
  socket.message(JSON.stringify({protocol:'neko.remoted.v1', version:1}));
  await result;
  assert.equal(socket.closed, true);
  assert.equal(socket.onmessage, null);
});

test('invalid, binary, incompatible and oversized responses fail closed', async () => {
  for (const data of ['bad json', 'null', '{}', JSON.stringify({protocol:'neko.remoted.v1',version:2}), new Uint8Array(2), ' '.repeat(1025), '橘'.repeat(500)]) {
    const result = checkRelayAddress('wss://relay.example', {WebSocket: Socket});
    const socket = Socket.instances.at(-1);
    socket.message(data);
    await assert.rejects(result, failure('response'));
    assert.equal(socket.closed, true);
  }
});

test('network errors, premature closure and eight-second timeout stop the check', async t => {
  for (const event of ['onerror', 'onclose']) {
    const result = checkRelayAddress('https://relay.example', {WebSocket: Socket});
    Socket.instances.at(-1)[event]();
    await assert.rejects(result, failure('connection'));
  }
  t.mock.timers.enable({apis:['setTimeout']});
  const result = checkRelayAddress('https://relay.example', {WebSocket: Socket});
  t.mock.timers.tick(7999);
  assert.equal(Socket.instances.at(-1).closed, undefined);
  t.mock.timers.tick(1);
  await assert.rejects(result, failure('timeout'));
  assert.equal(Socket.instances.at(-1).closed, true);
});
