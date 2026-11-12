#!/usr/bin/python3
"""Isolated framed peer. Every connection shares only this test's scratch files."""
import fcntl
import json
import os
import select
import socket
import struct
import sys
import time
from pathlib import Path

inp = open(0, 'rb', buffering=0, closefd=False)
root = Path(os.environ['XDG_STATE_HOME'])
state_file = root / 'fixture.json'
log_file = root / 'requests.jsonl'
mode_file = root / 'mode'

def mode():
    return mode_file.read_text().strip() if mode_file.exists() else ''

def state(change=None):
    with (root / 'fixture.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        value = json.loads(state_file.read_text()) if state_file.exists() else {'core_id': 'fixture-core', 'terminals': [], 'next': 1}
        if change:
            change(value)
            state_file.write_text(json.dumps(value))
        return value

def log(value):
    with log_file.open('a') as stream:
        stream.write(json.dumps(value) + '\n')

def frame(channel, kind, data):
    if not isinstance(data, bytes):
        data = json.dumps(data).encode()
    sys.stdout.buffer.write(struct.pack('<IBI', channel, kind, len(data)) + data)
    sys.stdout.buffer.flush()

def exact(count):
    result = b''
    while len(result) < count:
        part = inp.read(count - len(result))
        if not part:
            return None
        result += part
    return result

endpoint = socket.socket(socket.AF_UNIX)
try:
    endpoint.bind(os.environ['HC_SOCKET'])
except OSError:
    pass
hello = json.loads(inp.readline())
methods = ['server.status', 'terminal.run', 'terminal.list', 'terminal.get', 'terminal.read', 'terminal.kill', 'terminal.attach', 'events.subscribe']
result = {'generation': 1, 'framed': True, 'features': ['attach', 'framed', 'status'], 'methods': methods, 'version': '0.1.20', 'core': {'id': state()['core_id']}}
sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': hello['id'], 'result': result}) + '\n')
sys.stdout.flush()
attached = None
closed = False
sequence = 10
while True:
    if attached and not closed:
        terminal = next((t for t in state()['terminals'] if t['address'] == attached), None)
        if terminal and terminal['state'] == 'finished':
            frame(1, 2, terminal)
            closed = True
    # BufferedReader can already contain a second pipe write. poll the actual pipe
    # only after all currently buffered bytes have been consumed one frame at a time.
    readable, _, _ = select.select([inp], [], [], .03)
    if not readable:
        continue
    header = exact(9)
    if header is None:
        break
    channel, kind, length = struct.unpack('<IBI', header)
    data = exact(length)
    if data is None:
        break
    if channel:
        log({'kind': kind, 'channel': channel, 'data': list(data)})
        if kind == 0:
            frame(1, 0, b'echo:' + data)
        if kind == 4 and mode() in ('holder_lost', 'unknown_exit'):
            reason = 'holder_lost' if mode() == 'holder_lost' else 'exited'
            frame(1, 2, {'reason': reason})
            closed = True
        if kind == 4 and mode() == 'worker_lost':
            sys.exit(0)
        if kind == 3 and mode() == 'write_lost':
            sys.exit(0)
        continue
    request = json.loads(data)
    log(request)
    method, params = request['method'], request['params']
    value = state()
    if method == 'terminal.list':
        labels = params.get('labels', {})
        reply = {'terminals': [t for t in value['terminals'] if all(t['labels'].get(k) == v for k, v in labels.items())]}
    elif method == 'terminal.run':
        saved = json.loads((root / 'host-saved.json').read_text())
        assert any(r['grant'] == params['labels']['neoko_session'] and r['address'] is None for r in saved['hc']['terminals']), 'run was sent without a durable intent'
        terminal = {'address': 't' + str(value['next']), 'labels': params['labels'], 'state': 'running', 'keep': True, 'status': {'state': 'working', 'app': 'fixture'}, 'records': {}}
        def created(st):
            st['next'] += 1
            st['terminals'].append(terminal)
        state(created)
        if mode() == 'run_lost':
            sys.exit(0)
        reply = terminal
    elif method == 'terminal.attach':
        attached = params['terminal']
        terminal = next(t for t in value['terminals'] if t['address'] == attached)
        # An early first paint before the JSON-RPC result is deliberately legal.
        frame(1, 0, b'initial\x1b[6n\r\n')
        reply = {'channel': 1, 'terminal': terminal}
    elif method == 'events.subscribe':
        terminal = next(t for t in value['terminals'] if t['address'] == params['terminal'])
        if mode() == 'oversized_status':
            terminal['records'] = {f'worker-{n}': {'state': 'working', 'msg': 'M' * 2048} for n in range(63)}
        reply = {'channel': 2, 'seq': sequence, 'terminals': [terminal]}
    elif method == 'terminal.read':
        reply = {'text': 'final screen\n', 'complete': True}
    elif method == 'terminal.kill':
        def killed(st):
            for t in st['terminals']:
                if t['address'] == params['terminal']:
                    t.update(state='finished', reason='killed', exit_code=7)
                    if mode() == 'kill_unknown':
                        t.pop('exit_code', None)
        value = state(killed)
        reply = next(t for t in value['terminals'] if t['address'] == params['terminal'])
    else:
        raise AssertionError(method)
    frame(0, 0, {'jsonrpc': '2.0', 'id': request['id'], 'result': reply})
    if method == 'events.subscribe':
        if mode() == 'oversized_status':
            continue
        sequence += 1
        # Event lines cross DATA boundaries; old sequence and unchanged tables
        # must not produce duplicate current-status notifications.
        event = (json.dumps({'method': 'event', 'params': {'seq': sequence, 'terminal': attached, 'type': 'status', 'data': {'status': {'state': 'done', 'app': 'fixture'}}}}) + '\n').encode()
        frame(2, 0, event[:9])
        frame(2, 0, event[9:])
        if mode() == 'burst':
            for _ in range(512):
                frame(1, 0, b'X' * 16384)

        if mode() == 'attach_lost':
            log({'disconnected': True})
            sys.exit(0)
