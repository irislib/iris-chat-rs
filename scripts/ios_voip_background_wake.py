#!/usr/bin/env python3
"""One production VoIP OS wake, with private physical lifecycle evidence.

No phone, credential access or network request occurs on import. Execution
requires reviewed current token ownership, qualified source and an OwnedWindow.
APNs acceptance alone is never proof of a wake or a release acceptance.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

import ios_relaunch_lifecycle as lifecycle
from ios_ordinary_relaunch import (binding, exact, finite, process, read_bound, require,
    validate_distribution, validate_snapshot, verify_binding, HOST_CLOCK_TOLERANCE_SECONDS)
from ios_relaunch_adapters import Capture, CoreDevice, OwnedWindow
from ios_ostrace_capture import OstraceCapture, validate_evidence as validate_trace_evidence

SETTINGS = 'com.apple.Preferences'
BACKGROUND = 'running-active-NotVisible'
SOURCE_NAMES = ('ios_voip_background_wake.py', 'test_ios_voip_background_wake.py',
    'ios_ostrace_logger.py', 'ios_ostrace_capture.py', 'test_ios_ostrace_capture.py',
    'ios_relaunch_adapters.py', 'ios_relaunch_lifecycle.py', 'ios_ordinary_relaunch.py',
    'ios_runtime_acceptance.py', 'native_lab.py', 'test_ios_relaunch_adapters.py',
    'test_ios_relaunch_lifecycle.py', 'test_ios_ordinary_relaunch.py', 'test_ios_runtime_acceptance.py')
TESTS = ('test_ios_ostrace_capture', 'test_ios_voip_background_wake', 'test_ios_relaunch_adapters',
         'test_ios_relaunch_lifecycle', 'test_ios_ordinary_relaunch', 'test_ios_runtime_acceptance')

# Built-in Node modules only. The pure entry point is exercised with fake I/O;
# production supplies the real built-ins below, without a selectable endpoint.
NODE_SENDER = r'''
function sendOnce(input, io) {
  return new Promise((resolve) => {
    let client, stream, timer, ended = false, requests = 0, connections = 0;
    let status = null, responseID = null, body = Buffer.alloc(0);
    function finish(failure) {
      if (ended) return;
      ended = true;
      if (timer) io.clearTimeout(timer);
      if (stream) stream.destroy();
      if (client) client.destroy();
      resolve({schema:'iris-apns-single-request-v1', accepted:failure === null,
        failure, http_status:status, apns_id:responseID, request_count:requests,
        connection_count:connections, retry_count:0, device_wake_verified:false});
    }
    try {
      const keys = ['token','topic','key_path','key_sha256','key_id','team_id','request_id','issued_at'];
      if (Object.keys(input).sort().join() !== keys.sort().join()
          || !/^[0-9a-f]{64,512}$/.test(input.token) || input.token.length % 2
          || !/^[A-Za-z0-9.-]+\.voip$/.test(input.topic)
          || !/^[A-Z0-9]{10}$/.test(input.key_id) || !/^[A-Z0-9]{10}$/.test(input.team_id)
          || !/^[0-9a-f]{64}$/.test(input.key_sha256)
          || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(input.request_id)
          || !Number.isInteger(input.issued_at) || Math.abs(io.now() - input.issued_at) > 30)
        return finish('invalid-input');
      const pem = io.readKey(input.key_path);
      if (io.hash(pem) !== input.key_sha256) return finish('key-changed');
      const key = io.createPrivateKey(pem);
      if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails.namedCurve !== 'prime256v1')
        return finish('wrong-key-type');
      const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
      const unsigned = encode({alg:'ES256',kid:input.key_id}) + '.' + encode({iss:input.team_id,iat:input.issued_at});
      const signature = io.sign(Buffer.from(unsigned), key);
      if (signature.length !== 64) return finish('invalid-signature-format');
      const bearer = unsigned + '.' + signature.toString('base64url');
      // No Nostr event, caller, recipient identity, call invitation or message.
      // PushKit reports/ends an unavailable call through the production app.
      const payload = JSON.stringify({aps:{'content-available':1},iris_unavailable_call:input.request_id});
      timer = io.setTimeout(() => finish('deadline'), 20000);
      connections++;
      client = io.connect('https://api.push.apple.com:443', {
        rejectUnauthorized:true, ALPNProtocols:['h2'], maxSessionMemory:1,
        maxHeaderListPairs:32, settings:{enablePush:false,maxConcurrentStreams:1,maxHeaderListSize:8192}});
      client.once('error', () => finish('session-error'));
      client.once('goaway', () => finish('goaway'));
      client.once('close', () => { if (!ended) finish('session-closed'); });
      client.once('stream', () => finish('unexpected-server-stream'));
      client.once('connect', () => {
        if (ended) return;
        if (client.alpnProtocol !== 'h2' || requests !== 0) return finish('wrong-transport');
        try {
          requests++;
          stream = client.request({':method':'POST', ':scheme':'https', ':authority':'api.push.apple.com',
            ':path':'/3/device/' + input.token, authorization:'bearer ' + bearer,
            'apns-topic':input.topic, 'apns-push-type':'voip', 'apns-priority':'10',
            'apns-expiration':'0', 'apns-id':input.request_id, 'content-type':'application/json'});
          stream.once('error', () => finish('stream-error'));
          stream.once('aborted', () => finish('stream-aborted'));
          stream.once('response', headers => { status = headers[':status']; responseID = headers['apns-id'] || null; });
          stream.on('data', data => {
            if (body.length + data.length > 8192) return finish('response-too-large');
            body = Buffer.concat([body,data]);
          });
          stream.once('end', () => finish(status === 200 && responseID === input.request_id && body.length === 0 ? null : 'apns-rejected'));
          stream.end(payload);
        } catch (_) { finish('request-error'); }
      });
    } catch (_) { finish('local-input-or-crypto-error'); }
  });
}
'''

NODE_MAIN = NODE_SENDER + r'''
const fs = require('node:fs'), crypto = require('node:crypto'), http2 = require('node:http2');
let chunks = [], length = 0;
process.stdin.on('data', chunk => {
  length += chunk.length;
  if (length > 16384) { process.stderr.write('input-too-large\n'); process.exit(1); }
  chunks.push(chunk);
});
process.stdin.on('end', async () => {
  try {
    const input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    const result = await sendOnce(input, {
      now:() => Math.floor(Date.now()/1000), readKey:path => fs.readFileSync(path),
      hash:data => crypto.createHash('sha256').update(data).digest('hex'),
      createPrivateKey:data => crypto.createPrivateKey(data),
      sign:(data,key) => crypto.sign('sha256',data,{key,dsaEncoding:'ieee-p1363'}),
      connect:(host,options) => http2.connect(host,options), setTimeout, clearTimeout});
    process.stdout.write(JSON.stringify(result)+'\n');
    process.exitCode = result.accepted ? 0 : 1;
  } catch (_) { process.stderr.write('invalid-private-input\n'); process.exitCode=1; }
});
'''


def sources():
    return [binding(Path(__file__).with_name(name)) for name in SOURCE_NAMES]


def save(path, value):
    with Path(path).open('x') as output:
        json.dump(value, output, sort_keys=True, indent=2, allow_nan=False)
        output.write('\n')


def qualified_sources(config):
    record = read_bound(config['qualification'])
    current = sources()
    require(record.get('schema') == 'iris-voip-wake-qualification-v1' and record.get('passed') is True
            and record.get('exit_code') == 0 and record.get('device_actions') is False
            and record.get('network_requests') == 0 and record.get('checker_sources') == current
            and record.get('tests') == list(TESTS), 'VoIP wake source is not qualified')
    verify_binding(record['tests_log'])
    require(record.get('node') == config['node'], 'Qualified Node runtime differs')
    require(verify_binding(record['tagged_validator']).read_bytes()
            == Path(__file__).with_name('ios_runtime_acceptance.py').read_bytes(), 'Tagged v3 validator changed')
    return current


def token_for_owned_phone(config, window, *, now):
    """Consume a reviewed exact identity/continuity chain and server point read.

    Ownership and its freshness window are release-owner decisions grounded in
    the bound private evidence. An app-container extraction is not mandatory.
    """
    value = read_bound(config['token_evidence'])
    exact(value, {'schema','candidate','device','bundle_id','subscription_id','inputs','review'})
    require(value['schema'] == 'iris-owned-voip-token-v1' and value['candidate'] == window['candidate']
            and value['device'] == window['device'] and value['bundle_id'] == window['bundle_id']
            and isinstance(value['subscription_id'],str) and re.fullmatch(r'[A-Za-z0-9-]+',value['subscription_id']),
            'Wrong owned-phone subscription identity')
    exact(value['inputs'], {'identity_evidence','phone_continuity','installed_inventory','subscription','lookup'})
    for name in ('identity_evidence','phone_continuity'):
        chain = value['inputs'][name]
        require(isinstance(chain,list) and 1 <= len(chain) <= 16, 'Missing bounded ownership evidence chain')
        for source in chain: verify_binding(source)
    for name in ('installed_inventory','subscription','lookup'): verify_binding(value['inputs'][name])
    review = read_bound(value['review'])
    exact(review, {'schema','decision','candidate','device','bundle_id','subscription_id','inputs',
                   'valid_from_utc','valid_until_utc','ownership_basis','token_sha256'})
    require(review['schema'] == 'iris-owned-voip-token-review-v1' and review['decision'] == 'accepted'
            and all(review[k] == value[k] for k in ('candidate','device','bundle_id','subscription_id','inputs'))
            and review['ownership_basis'] in {'current-registration','retained-registration-continuity'}
            and finite(review['valid_from_utc']) <= finite(now) <= finite(review['valid_until_utc'])
            and review['valid_until_utc'] > review['valid_from_utc'],
            'Exact phone/token ownership has not been reviewed')
    subscription_id = value['subscription_id']
    apps = read_bound(value['inputs']['installed_inventory'])['result']['apps']
    require(len(apps) == 1 and apps[0].get('bundleIdentifier') == window['bundle_id']
            and apps[0].get('bundleVersion') == window['build']
            and apps[0].get('url') + window['process_name'] == window['executable'], 'Token inventory is another app/build')
    lookup = read_bound(value['inputs']['lookup'])
    require(lookup.get('schema') == 'iris-exact-subscription-read-v1'
            and lookup.get('subscription_id') == subscription_id and lookup.get('response') == value['inputs']['subscription']
            and lookup.get('read_only') is True and lookup.get('record_count') == 1
            and lookup.get('scope') == 'current-owned-phone-call-subscription'
            and review['valid_from_utc'] <= finite(lookup.get('observed_utc')) <= now,
            'Lookup did not bind exactly the owned subscription within the reviewed window')
    subscription = read_bound(value['inputs']['subscription'])
    tokens = subscription.get('apns_tokens')
    require(isinstance(tokens, list) and len(tokens) == 1 and isinstance(tokens[0], str)
            and re.fullmatch(r'[0-9a-f]{64,512}', tokens[0]) and len(tokens[0]) % 2 == 0
            and subscription.get('apns_environment') == 'production'
            and subscription.get('apns_topic') == window['bundle_id'] + '.voip'
            and subscription.get('filter', {}).get('kinds') == [21111]
            and subscription.get('filters') == [], 'Ambiguous, nonproduction or non-call APNs registration')
    require(review['token_sha256'] == hashlib.sha256(tokens[0].encode()).hexdigest(), 'Reviewed token differs from server read')
    return tokens[0]


class SendOnce:
    def __init__(self, window, config):
        self.window, self.config, self.attempted = window, config, False
        self.item = None

    def start(self):
        require(not self.attempted, 'A push request cannot be retried')
        self.attempted = True
        window, config = self.window, self.config
        window.guard()
        token = token_for_owned_phone(config, window.config, now=time.time())
        credentials = read_bound(config['credentials'])
        require(credentials.get('schema') == 'iris-existing-apns-credentials-v1'
                and credentials.get('environment') == 'production'
                and credentials.get('bundle_id') == window.config['bundle_id']
                and credentials.get('remote_key_sha256') == credentials.get('local_key', {}).get('sha256'),
                'APNs signing material differs from established production credentials')
        key = verify_binding(credentials['local_key'])
        require(key.stat().st_mode & 0o077 == 0, 'Signing key is not owner-only')
        node = verify_binding(config['node'])
        self.request_id = str(uuid.uuid4())
        private_input = {'token':token, 'topic':window.config['bundle_id']+'.voip',
            'key_path':str(key), 'key_sha256':credentials['local_key']['sha256'],
            'key_id':credentials['key_id'], 'team_id':credentials['team_id'],
            'request_id':self.request_id, 'issued_at':int(time.time())}
        # No inherited preload modules, TLS overrides, proxy or debug-key logging.
        window.environment = {k:v for k,v in window.environment.items() if not k.upper().startswith(
            ('NODE_', 'OPENSSL_', 'SSL_', 'SSLKEYLOGFILE', 'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY'))}
        self.started = {'started_utc':time.time(), 'started_monotonic':time.monotonic()}
        self.item = window.spawn([str(node), '--no-addons', '--disable-proto=throw', '-e', NODE_MAIN], 'apns', stdin=True)
        self.item['process'].stdin.write(json.dumps(private_input, separators=(',', ':')).encode())
        self.item['process'].stdin.close()
        return self.started

    def finish(self):
        require(self.item is not None and self.item['process'].poll() is not None, 'Push sender has not exited')
        require(time.monotonic() - self.started['started_monotonic'] <= 25, 'Push sender exceeded deadline')
        require(not self.item['stderr'].read_bytes() and self.item['stdout'].stat().st_size <= 4096, 'Sender diagnostic or oversized output')
        result = json.loads(self.item['stdout'].read_text())
        exact(result, {'schema','accepted','failure','http_status','apns_id','request_count','connection_count','retry_count','device_wake_verified'})
        require(result['schema'] == 'iris-apns-single-request-v1' and type(result['retry_count']) is int
                and result['retry_count'] == 0 and type(result['accepted']) is bool
                and all(type(result[k]) is int and result[k] in (0, 1) for k in ('request_count','connection_count'))
                and result['device_wake_verified'] is False, 'Invalid single-request sender evidence')
        return {**self.started, 'finished_utc':time.time(), 'finished_monotonic':time.monotonic(),
            'result':result, 'request_id':self.request_id, 'returncode':self.item['process'].returncode,
            'stdout':binding(self.item['stdout']), 'stderr':binding(self.item['stderr']),
            'token_evidence':self.config['token_evidence'], 'credentials':self.config['credentials'], 'node':self.config['node']}


def start_capture(capture):
    """Use Capture's live/cutoff/closure checks, also observing Settings states."""
    require(not capture.streams, 'Capture already started')
    window, c = capture.window, capture.window.config
    rows = capture.core.run(['device', 'info', 'processes'])['runningProcesses']
    boards = [r for r in rows if r.get('executable', '').endswith('/SpringBoard.app/SpringBoard')]
    require(len(boards) == 1, 'Missing unique SpringBoard process')
    capture.os_process = boards[0]
    filters = {'startup': ['-m', 'iris.startup '],
               'lifecycle': ['-p', str(boards[0]['processIdentifier']), '-m', c['bundle_id'], '-m', SETTINGS]}
    for name, selection in filters.items():
        path = window.out / (name + '.log')
        item = window.spawn([str(window.tools['syslog']), '-u', c['device'], '--no-colors',
                             *selection, '-o', str(path)], name)
        item.update(capture=path, capture_name=name)
        capture.streams[name] = item
    deadline = min(window.clock() + 5, c['deadline_monotonic'])
    while True:
        window.guard(); capture.health()
        if all(x['capture'].exists() and x['capture'].read_bytes().startswith(('[connected:' + c['device'] + ']\n').encode())
               for x in capture.streams.values()):
            capture.prefixes()
            return
        require(window.clock() < deadline, 'Loggers did not become ready')
        time.sleep(.02)



def options(result, coredevice_id, *, activated, replacement):
    require(result.get('deviceIdentifier') == coredevice_id, 'Wrong launch device')
    value = result.get('launchOptions')
    exact(value, {'activatedWhenStarted', 'arguments', 'environmentVariables', 'platformSpecificOptions',
                  'startStopped', 'terminateExistingInstances', 'user'})
    require(value['activatedWhenStarted'] is activated and value['terminateExistingInstances'] is replacement
            and value['startStopped'] is False and value['arguments'] == []
            and value['environmentVariables'] in ({}, {'TERM': 'dumb'})
            and value['platformSpecificOptions'] == {} and value['user'] == {'active': True},
            'Nonordinary launch options')



def identity(result, executable):
    row = result.get('process', {})
    pid, audit = row.get('processIdentifier'), row.get('auditToken')
    process({'pid': pid, 'executable': row.get('executable')})
    require(row['executable'] == executable and isinstance(audit, list) and len(audit) == 8
            and all(type(n) is int and 0 <= n <= 2**32 - 1 for n in audit) and audit[5] == pid,
            'Invalid actual launch identity')
    return {'pid': pid, 'executable': executable, 'audit_token': audit}



def state_records(data, initial, bundle, board_pid):
    pattern = re.compile(r'SpringBoard(?:\(RunningBoardServices\))?\[' + str(board_pid)
        + r'\] <[^>]+>: Received state update for (\d+) \(app<('
        + re.escape(bundle) + '|' + re.escape(SETTINGS) + r')(?:\([^>]*\))?>, ([A-Za-z-]+)(?:[ ,].*)?$')
    def parse(line):
        if 'Received state update for ' not in line or not any(x in line for x in (bundle, SETTINGS)):
            return None
        match = pattern.search(line)
        require(match is not None, 'Unrecognized lifecycle record or changed SpringBoard PID')
        return {'pid': int(match[1]), 'bundle': match[2], 'state': match[3]}
    return lifecycle._records(data, initial, parse)



def background_events(data, initial, bundle, board_pid, settings_pid, pid, old_pid):
    """Fail on any foreground activation, even if it falls between samples."""
    prior = state_records(data[:initial], 0, bundle, board_pid)
    settings = [r for r in prior if r['bundle'] == SETTINGS]
    require(settings and settings[-1]['pid'] == settings_pid and settings[-1]['state'] == lifecycle.ACTIVE,
            'Settings was not observed active before cold launch')
    current, suspended = [], False
    for row in state_records(data, initial, bundle, board_pid):
        if row['bundle'] == SETTINGS:
            require(row['pid'] == settings_pid and row['state'] == lifecycle.ACTIVE,
                    'Settings ceased to be foreground during cold launch')
        elif row['pid'] == old_pid:
            require(not current and row['state'] in {'not-running', 'terminated', 'exited'},
                    'Old process changed state after replacement')
        else:
            require(row['pid'] == pid and row['state'] in {BACKGROUND, lifecycle.SUSPENDED},
                    'Cold launch entered foreground, exited or changed identity')
            require(not suspended or row['state'] == lifecycle.SUSPENDED, 'Cold suspension was interrupted')
            suspended = suspended or row['state'] == lifecycle.SUSPENDED
            current.append(row)
    return current



def resumed_visible_events(rows, pid):
    """Allow the normal activation transition; time only continuous visibility."""
    visible = []
    for row in rows:
        require(row['pid'] == pid and row['state'] in {lifecycle.SUSPENDED, BACKGROUND, lifecycle.ACTIVE},
                'Resumed target exited or changed identity')
        require(not visible or row['state'] == lifecycle.ACTIVE, 'Resumed target left foreground')
        if row['state'] == lifecycle.ACTIVE:
            visible.append(row)
    return visible


def evaluate_lifecycle(capture, initial_prefixes, resume_index, *, bundle, board_pid, settings_pid, proof):
    raw, measured = lifecycle.measurement_captures(**{k: capture[k] for k in
        ('startup_text', 'lifecycle_text', 'samples', 'measurement_cutoff')}, device=capture['device'])
    samples = capture['samples']
    require(type(resume_index) is int and 0 <= resume_index < len(samples) - 1, 'Missing pre-resume sample')
    exact(initial_prefixes, {'startup', 'lifecycle'})
    for name, value in initial_prefixes.items():
        exact(value, {'bytes', 'sha256'})
        lifecycle._capture(lifecycle._prefix(measured[name], value['bytes'], value['sha256'], complete=True), capture['device'])
    boundary = samples[resume_index]
    cold_data = measured['lifecycle'][:boundary['lifecycle_bytes']]
    verify_os_bootstrap(measured['lifecycle'], initial_prefixes['lifecycle']['bytes'], bundle, board_pid, proof)
    events = background_events(cold_data, initial_prefixes['lifecycle']['bytes'], bundle, board_pid,
                               settings_pid, proof['pid'], proof['previous_pid'])
    require(events and events[-1]['state'] == lifecycle.SUSPENDED, 'No actual cold suspension')
    suspended_device = next(r['stamp'] for r in events if r['state'] == lifecycle.SUSPENDED)
    target_rows = []
    for row in state_records(measured['lifecycle'], boundary['lifecycle_bytes'], bundle, board_pid):
        if row['bundle'] == SETTINGS:
            require(row['pid'] == settings_pid and row['state'] in {lifecycle.ACTIVE, BACKGROUND, lifecycle.SUSPENDED},
                    'Settings process changed during resume')
            continue
        target_rows.append(row)
    resumed = resumed_visible_events(target_rows, proof['pid'])
    require(resumed and resumed[0]['stamp'] - suspended_device >= 135, 'Device suspension was shorter than 135 seconds')
    marker = re.compile(r'(?:^| )' + re.escape(capture['process_name'])
        + r'(?:\([^)]*\))?\[(\d+)\] <[^>]+>: iris\.startup phase=(\w+) '
          r'restore_pending_result=(true|false) native_restoring_observed=(true|false) app_backgrounded=(true|false)$')
    def parse_marker(line):
        if 'iris.startup ' not in line:
            return None
        match = marker.search(line)
        require(match is not None and int(match[1]) == proof['pid'], 'Wrong startup marker process')
        return {'phase': match[2], 'pending': match[3] == 'true', 'background': match[5] == 'true'}
    startup = lifecycle._records(measured['startup'], initial_prefixes['startup']['bytes'], parse_marker)
    core = [r for r in startup if r['phase'] != 'backgrounded']
    require([r['phase'] for r in core] == ['restore_enqueued', 'restore_result_observed']
            and len(startup) - len(core) <= 1 and core[0]['pending'] and not core[1]['pending']
            and core[0]['stamp'] < core[1]['stamp']
            and all(r['background'] and r['end_byte'] <= boundary['startup_bytes'] for r in startup),
            'Restoration did not complete in the cold background process')
    previous_time, sizes = None, {k: v['bytes'] for k, v in initial_prefixes.items()}
    suspended_start = suspended_last = resumed_start = resumed_last = None
    for index, row in enumerate(samples):
        exact(row, lifecycle.SAMPLE_KEYS)
        now = finite(row['observed_monotonic'])
        require(previous_time is None or 0 < now - previous_time <= lifecycle.MAX_SAMPLE_GAP_SECONDS, 'Observation gap or reversed clock')
        previous_time = now
        require(row['pid'] == proof['pid'] and type(row['pid']) is int and row['executable'] == proof['executable']
                and row['loggers_alive'] is True and row['stderr'] == '', 'Process identity or logger health changed')
        for name, data in measured.items():
            lifecycle._prefix(data, row[name + '_bytes'], row[name + '_sha256'], complete=True)
            require(row[name + '_bytes'] >= sizes[name], 'Capture shrank')
            sizes[name] = row[name + '_bytes']
        if index <= resume_index:
            visible = [r for r in events if r['end_byte'] <= row['lifecycle_bytes']]
            if visible and visible[-1]['state'] == lifecycle.SUSPENDED:
                if suspended_start is None: suspended_start = now
                suspended_last = now
        elif any(r['end_byte'] <= row['lifecycle_bytes'] for r in resumed):
            if resumed_start is None: resumed_start = now
            resumed_last = now
    require(suspended_start is not None and suspended_last - suspended_start >= 135,
            'Fewer than 135 live observed seconds suspended')
    require(resumed_start is not None and resumed_last - resumed_start >= 15, 'Missing 15 live resumed seconds')
    require(samples[-1]['observed_monotonic'] - samples[0]['observed_monotonic'] <= lifecycle.MAX_OBSERVATION_SECONDS
            and all(sizes[name] == len(data) for name, data in measured.items()), 'Incomplete or excessive observation')
    return {'entry_state': 'background', 'foreground_activations': 0,
            'observed_seconds': suspended_last - suspended_start, 'resumed_seconds': resumed_last - resumed_start,
            'process_samples': len(samples), 'unexpected_exits': 0, 'process_replacements': 0,
            'suspended_last_monotonic': suspended_last, 'resumed_started_monotonic': resumed_start,
            'raw_sha256': {name: hashlib.sha256(data).hexdigest() for name, data in raw.items()},
            'physical_release_acceptance': False, 'preserved_state_followup_required': True}


def verify_os_bootstrap(data, initial, bundle, board_pid, proof):
    pattern = re.compile(r'SpringBoard(?:\(FrontBoard\))?\[' + str(board_pid)
        + r'\] <[^>]+>: \[app<' + re.escape(bundle) + r'(?:\([^>]*\))?>:(\d+)\] Bootstrap success!$')
    def parse(line):
        if bundle not in line:
            return None
        require(not ('Bootstrapping ' in line and 'foreground' in line.lower()), 'OS requested a foreground bootstrap')
        require(not ('Process exited' in line and re.search(r':' + str(proof['pid']) + r'(?:\D|$)', line)),
                'Cold process exited between inventories')
        if 'Bootstrap success!' not in line:
            return None
        match = pattern.search(line)
        require(match is not None and int(match[1]) == proof['pid'], 'Ambiguous OS bootstrap or changed OS source')
        return {'pid':int(match[1])}
    boots = lifecycle._records(data, initial, parse)
    require(len(boots) == 1, 'Need exactly one successful OS bootstrap')
    states = [row for row in state_records(data, initial, bundle, board_pid)
              if row['bundle'] == bundle and row['pid'] == proof['pid']]
    suspended = [row for row in states if row['state'] == lifecycle.SUSPENDED]
    # FrontBoard can report initial background state before bootstrap completion.
    # Both must identify the new process before its first actual suspension.
    require(states and suspended and states[0]['state'] in {BACKGROUND, lifecycle.SUSPENDED}
            and boots[0]['stamp'] <= suspended[0]['stamp'], 'OS bootstrap did not enter background')


class WakeCoreDevice(CoreDevice):
    """Additional operations are only Settings and one admitted old target PID."""
    def __init__(self, window, existing_pid):
        super().__init__(window)
        require(type(existing_pid) is int and existing_pid > 0, 'Missing admitted old PID')
        self.existing_pid = existing_pid
        self.terminated = False

    def run(self, command):
        terminate = ['device', 'process', 'terminate', '--pid', str(self.existing_pid)]
        if command not in (['device', 'process', 'launch', SETTINGS], terminate):
            return super().run(command)
        if command == terminate:
            require(not self.terminated, 'Owned termination cannot be retried')
            self.terminated = True
        self.window.guard()
        output = self.window.out / f'coredevice-{self.window.serial + 1:03d}.json'
        args = [str(self.window.tools['coredevice']), *command[:3], '--device', self.window.config['device'],
                '--timeout', '6', '--json-output', str(output), *command[3:]]
        item = self.window.spawn(args, 'coredevice')
        self.window.wait(item, 7)
        require(output.stat().st_size <= 4 * 1024 * 1024, 'Oversized CoreDevice response')
        return json.loads(output.read_text())['result']


def inventory_binding(window):
    return binding(window.out / f'coredevice-{window.serial:03d}.json')


def operation(core, command):
    value = {'command':command, 'started_utc':time.time(), 'started_monotonic':time.monotonic()}
    value['result'] = core.run(command)
    value.update(returned_utc=time.time(), returned_monotonic=time.monotonic(), raw=inventory_binding(core.window))
    return value


def validate_resume(distribution, resume, proof):
    require(resume['command'] == ['device','process','launch',distribution['bundle_id']], 'Resume was not ordinary')
    require(distribution['installation_finished_utc'] <= finite(resume['started_utc'])
            <= finite(resume['returned_utc']) <= finite(resume['validated_utc'])
            and resume['validated_utc'] - resume['returned_utc'] <= 8, 'Stale resume result')
    options(resume['result'], distribution['coredevice_id'], activated=True, replacement=False)
    actual = identity(resume['result'], proof['executable'])
    require(actual['pid'] == proof['pid'], 'Resume replaced the cold process')
    if proof['initial_audit_token'] is not None:
        require(actual['audit_token'] == proof['initial_audit_token'], 'Resume changed actual audit identity')
    snapshot = resume['snapshot']
    current = validate_snapshot(snapshot, proof['executable'], now=resume['validated_utc'])
    require(current['pid'] == proof['pid'] and snapshot['observed_utc'] >= resume['returned_utc'], 'Resume inventory mismatch')
    return {**actual, 'initial_audit_token_available':proof['initial_audit_token'] is not None,
            'initial_to_resume_audit_equality_claimed':proof['initial_audit_token'] is not None}


def evaluate(record):
    """Recheck native raw evidence; initial audit absence is explicit for OS wake."""
    config = read_bound(record['configuration'])
    window = read_bound(config['window'])
    distribution = validate_distribution(window['distribution_binding'], window['candidate'])
    require(all(window[k] == distribution[k] for k in ('device','coredevice_id','bundle_id'))
            and window['build'] == distribution['candidate']['apple_build']
            and window['executable'] == distribution['installed_url'] + window['process_name'],
            'Observation target differs from qualified distribution')
    require(qualified_sources(config) == record['checker_sources'], 'Qualified source changed')
    require(record['automatic_retry'] is False and record['push_attempts'] == 1
            and type(record['push_attempts']) is int, 'Not exactly one attempted push')
    require(record['absent']['processes'] == [] and record['absent']['foreground_pid'] is None,
            'App was not absent before push')
    require(0 <= record['push']['started_utc'] - finite(record['absent']['observed_utc']) <= 8,
            'Absent baseline is stale')
    absent_rows = read_bound(record['absent_inventory'])['result']['runningProcesses']
    require(not any(r.get('executable') == window['executable'] or '-Runner.app/' in r.get('executable','') for r in absent_rows),
            'Actual baseline contains Iris or XCTest')
    push = record['push']
    require(read_bound(push['stdout']) == push['result'] and not verify_binding(push['stderr']).read_bytes(),
            'Sender result changed or reported a diagnostic')
    require(push['result']['accepted'] is True and push['result']['http_status'] == 200
            and push['result']['apns_id'] == push['request_id'] and push['result']['request_count'] == 1
            and push['result']['connection_count'] == 1 and push['result']['retry_count'] == 0
            and push['result']['device_wake_verified'] is False and push['returncode'] == 0,
            'APNs did not accept one request')
    require(push['token_evidence'] == config['token_evidence'] and push['credentials'] == config['credentials']
            and push['node'] == config['node'], 'Push used different private inputs')
    token_for_owned_phone(config, window, now=push['started_utc'])
    for item in (config['credentials'], config['node']): verify_binding(item)
    apps = read_bound(record['apps'])['result']['apps']
    require(len(apps) == 1 and apps[0].get('bundleIdentifier') == window['bundle_id']
            and apps[0].get('bundleVersion') == window['build']
            and apps[0].get('url') == distribution['installed_url'], 'Installed wake candidate differs')
    proof = record['process']
    exact(proof, {'pid','executable','previous_pid','initial_audit_token','first_observed_utc','first_observed_monotonic'})
    process({k:proof[k] for k in ('pid','executable')})
    require(proof['executable'] == window['executable'] and proof['previous_pid'] == config['existing_pid']
            and proof['pid'] != proof['previous_pid'] and proof['initial_audit_token'] is None
            and proof['first_observed_utc'] >= push['started_utc'], 'Cold identity is not an observed fresh OS process')
    termination = record['terminate']
    require(termination['command'] == ['device','process','terminate','--pid',str(config['existing_pid'])]
            and read_bound(termination['raw'])['result'] == termination['result']
            and termination['returned_utc'] <= record['absent']['observed_utc'], 'Missing exact owned termination')
    settings = record['settings']
    require(settings['command'] == ['device','process','launch',SETTINGS]
            and read_bound(settings['raw'])['result'] == settings['result'], 'Settings setup changed')
    options(settings['result'], window['coredevice_id'], activated=True, replacement=False)
    settings_exe = settings['result']['process']['executable']
    require(settings_exe.endswith('/Preferences.app/Preferences'), 'Setup did not activate Settings')
    settings_pid = identity(settings['result'], settings_exe)['pid']
    resume = record['resume']
    require(read_bound(resume['raw'])['result'] == resume['result'], 'Actual resume result changed')
    resumed = validate_resume(distribution, resume, proof)
    capture = record['capture']
    exact(capture, {'startup_text','lifecycle_text','process_name','samples','measurement_cutoff','closure_binding'})
    require(record['trace_capture']['transport'] == config['trace_transport'], 'Different trace transport')
    validate_trace_evidence(record['trace_capture'], capture, device=window['device'],
                            bundle=window['bundle_id'], board_pid=record['springboard_pid'])
    closure = read_bound(capture['closure_binding'])
    exact(closure, {'schema','closed','failures','candidate','device','groups','measurement_cutoff_sha256'})
    require(closure.get('schema') == 'iris-relaunch-adapter-closure-v1' and closure.get('closed') is True
            and closure.get('failures') == [] and closure.get('candidate') == window['candidate']
            and closure.get('device') == window['device'] and isinstance(closure.get('groups'),list)
            and len(closure['groups']) >= 3
            and closure.get('measurement_cutoff_sha256') == lifecycle.evidence_sha256(capture['measurement_cutoff']),
            'Owned capture did not close')
    require(all(type(pid) is int and pid > 0 for pid in closure['groups'])
            and len(set(closure['groups'])) == len(closure['groups']), 'Invalid owned process groups')
    require(capture['process_name'] == window['process_name'], 'Wrong startup identity')
    result = evaluate_lifecycle({**capture,'device':window['device']}, record['initial_prefixes'], record['resume_index'],
        bundle=window['bundle_id'], board_pid=record['springboard_pid'], settings_pid=settings_pid, proof=proof)
    samples = capture['samples']
    require(len(record['inventories']) == len(samples) and samples[0]['observed_monotonic'] >= push['started_monotonic']
            and push['started_monotonic'] <= finite(proof['first_observed_monotonic']) <= samples[0]['observed_monotonic']
            and samples[0]['observed_monotonic'] - proof['first_observed_monotonic'] <= lifecycle.MAX_SAMPLE_GAP_SECONDS
            and result['suspended_last_monotonic'] <= finite(resume['started_monotonic'])
            <= finite(resume['returned_monotonic']) <= result['resumed_started_monotonic'], 'Resume/observation order changed')
    offset = push['started_utc'] - push['started_monotonic']
    require(abs(proof['first_observed_utc'] - proof['first_observed_monotonic'] - offset)
            <= HOST_CLOCK_TOLERANCE_SECONDS, 'First observed identity clock changed')
    for item, boundaries in ((push, ('started','finished')), (resume, ('started','returned'))):
        for name in boundaries:
            require(abs(finite(item[name+'_utc']) - finite(item[name+'_monotonic']) - offset)
                    <= HOST_CLOCK_TOLERANCE_SECONDS, 'Host clock discontinuity')
    for source in [*record['inventories'], resume['inventory']]:
        rows = read_bound(source)['result']['runningProcesses']
        target = [r for r in rows if r.get('executable') == proof['executable']]
        require(len(target) == 1 and target[0].get('processIdentifier') == proof['pid']
                and not any('-Runner.app/' in r.get('executable','') for r in rows)
                and any(r.get('processIdentifier') == record['springboard_pid']
                        and r.get('executable','').endswith('/SpringBoard.app/SpringBoard') for r in rows),
                'Native inventory shows exit/replacement, XCTest or changed OS source')
    require(validate_distribution(window['distribution_binding'], window['candidate']) == distribution, 'Distribution changed')
    return {'schema':'iris-voip-background-runtime-v1','candidate':window['candidate'], 'lifecycle':result,
            'resume_identity':resumed, 'initial_audit_token_available':False,
            'process_identity_basis':'app-absent-baseline-one-os-bootstrap-continuous-pid-executable-lifecycle',
            'push_payload_correlation_claimed':False, 'preserved_state_followup_required':True,
            'physical_release_acceptance':False}


def observe(config_binding, output, *, execute=False):
    require(execute, 'Explicit reviewed VoIP wake execution required')
    config = read_bound(config_binding)
    exact(config, {'schema','window','node','credentials','token_evidence','qualification','existing_pid','trace_transport'})
    require(config['schema'] == 'iris-voip-background-input-v1'
            and type(config['existing_pid']) is int and config['existing_pid'] > 0, 'Invalid wake admission')
    checker_sources = qualified_sources(config)
    window = OwnedWindow(config['window'], output)
    record = {'schema':'iris-voip-background-capture-v1','configuration':config_binding,
              'checker_sources':checker_sources,'passed':False,'physical_release_acceptance':False,
              'push_attempts':0,'automatic_retry':False,'inventories':[]}
    capture, failure = None, None
    try:
        c = window.config
        token_for_owned_phone(config, c, now=time.time())
        verify_binding(config['credentials']); verify_binding(config['node'])
        core = WakeCoreDevice(window, config['existing_pid'])
        apps = core.run(['device','info','apps','--filter',"bundleIdentifier == '"+c['bundle_id']+"'"])['apps']
        record['apps'] = inventory_binding(window)
        require(len(apps) == 1 and apps[0].get('bundleIdentifier') == c['bundle_id']
                and apps[0].get('bundleVersion') == c['build']
                and apps[0].get('url') + c['process_name'] == c['executable'], 'Wrong installed candidate')
        capture = OstraceCapture(window, core, config['trace_transport'])
        capture.start()
        record['springboard_pid'] = capture.os_process['processIdentifier']
        before = capture.snapshot()
        expected = [{'pid':config['existing_pid'],'executable':c['executable']}]
        require(before['processes'] == expected, 'Admitted old process is absent')
        record['settings'] = operation(core, ['device','process','launch',SETTINGS])
        settings = record['settings']['result']
        options(settings, c['coredevice_id'], activated=True, replacement=False)
        settings_exe = settings['process']['executable']
        require(settings_exe.endswith('/Preferences.app/Preferences'), 'Setup did not activate Settings')
        settings_pid = identity(settings, settings_exe)['pid']
        deadline = time.monotonic() + 45
        while True:
            window.guard()
            snapshot = capture.snapshot()
            require(snapshot['processes'] == expected, 'Old process changed during setup')
            states = state_records(lifecycle._capture(capture.prefixes()['lifecycle_text'], c['device']),
                                   0, c['bundle_id'], record['springboard_pid'])
            other = [r for r in states if r['bundle'] == SETTINGS]
            iris = [r for r in states if r['bundle'] == c['bundle_id']]
            if other and other[-1]['pid'] == settings_pid and other[-1]['state'] == lifecycle.ACTIVE \
                    and iris and iris[-1]['pid'] == config['existing_pid'] and iris[-1]['state'] == lifecycle.SUSPENDED:
                break
            require(time.monotonic() < deadline, 'Settings/old-process suspension was not observed')
            time.sleep(.1)
        record['terminate'] = operation(core, ['device','process','terminate','--pid',str(config['existing_pid'])])
        deadline = time.monotonic() + 8
        while True:
            window.guard()
            absent = capture.snapshot()
            if not absent['processes']: break
            require(absent['processes'] == expected and time.monotonic() < deadline, 'Owned app did not terminate')
            time.sleep(.1)
        require(absent['foreground_pid'] is None, 'Iris remained visible after termination')
        record['absent'], record['absent_inventory'] = absent, inventory_binding(window)
        record['initial_prefixes'] = lifecycle.capture_prefixes(**capture.prefixes(), device=c['device'])
        sender = SendOnce(window, config)
        record['push_attempts'] = 1
        sender.start()
        record['push_started'] = sender.started
        deadline = time.monotonic() + 25
        while 'process' not in record or 'push' not in record:
            window.guard()
            require(time.monotonic() < deadline, 'No accepted request and unique OS wake before deadline')
            snapshot = capture.snapshot()
            require(len(snapshot['processes']) <= 1, 'Multiple target processes appeared')
            if snapshot['processes']:
                row = snapshot['processes'][0]
                require(row['pid'] != config['existing_pid'] and row['executable'] == c['executable'], 'OS wake reused old identity')
                if 'process' not in record:
                    record['process'] = {**row,'previous_pid':config['existing_pid'],'initial_audit_token':None,
                        'first_observed_utc':time.time(),'first_observed_monotonic':time.monotonic()}
                require(row['pid'] == record['process']['pid'], 'OS process changed before request completion')
                capture.sample(row['pid']); record['inventories'].append(inventory_binding(window))
                background_events(lifecycle._capture(capture.prefixes()['lifecycle_text'], c['device']),
                    record['initial_prefixes']['lifecycle']['bytes'], c['bundle_id'], record['springboard_pid'],
                    settings_pid, row['pid'], config['existing_pid'])
            elif 'process' in record:
                raise ValueError('OS process disappeared before request completion')
            if sender.item['process'].poll() is not None and 'push' not in record:
                record['push'] = sender.finish()
                require(record['push']['result']['accepted'] is True and record['push']['returncode'] == 0,
                        'Single APNs request was not accepted; no retry')
            if 'process' not in record or 'push' not in record: time.sleep(.25)
        proof = record['process']
        distribution = validate_distribution(c['distribution_binding'], c['candidate'])
        suspended_start = resumed_start = None
        deadline = time.monotonic() + 240
        while True:
            window.guard()
            require(time.monotonic() < deadline, 'Background observation deadline')
            row = capture.sample(proof['pid']); record['inventories'].append(inventory_binding(window))
            data = lifecycle._capture(capture.prefixes()['lifecycle_text'], c['device'])
            if 'resume' not in record:
                events = background_events(data, record['initial_prefixes']['lifecycle']['bytes'], c['bundle_id'],
                    record['springboard_pid'], settings_pid, proof['pid'], proof['previous_pid'])
                if events and events[-1]['state'] == lifecycle.SUSPENDED:
                    if suspended_start is None: suspended_start = row['observed_monotonic']
                    if row['observed_monotonic'] - suspended_start >= 136:
                        verify_os_bootstrap(data, record['initial_prefixes']['lifecycle']['bytes'],
                                            c['bundle_id'], record['springboard_pid'], proof)
                        record['resume_index'] = len(capture.samples) - 1
                        record['resume'] = operation(core, ['device','process','launch',c['bundle_id']])
                        record['resume']['snapshot'] = capture.snapshot()
                        record['resume']['inventory'] = inventory_binding(window)
                        record['resume']['validated_utc'] = time.time()
                        validate_resume(distribution, record['resume'], proof)
            else:
                rows = state_records(data, capture.samples[record['resume_index']]['lifecycle_bytes'],
                                     c['bundle_id'], record['springboard_pid'])
                iris = [r for r in rows if r['bundle'] == c['bundle_id']]
                visible = resumed_visible_events(iris, proof['pid'])
                if visible:
                    if resumed_start is None: resumed_start = row['observed_monotonic']
                    if row['observed_monotonic'] - resumed_start >= 16: break
            time.sleep(1.5)
        capture.mark_measurement_cutoff(capture.samples)
    except Exception as error:
        failure = error
        record['failure'] = str(error)
    finally:
        try:
            window.close()
        except Exception as error:
            record['closure_failure'] = str(error)
            if failure is None: failure = error
        if capture is not None:
            record['samples'], record['partial_cutoff'] = capture.samples, copy.deepcopy(capture.cutoff)
    if failure is None:
        try:
            record['capture'] = capture.final_capture(capture.samples)
            record['trace_capture'] = capture.trace_evidence
            require(read_bound(config_binding) == config and sources() == checker_sources, 'Execution inputs changed')
            record['evaluation'] = evaluate(record)
            record['passed'] = True
        except Exception as error:
            failure = error
            record['failure'] = str(error)
    save(window.out / 'capture.json', record)
    if failure is not None: raise failure
    return record['evaluation']


def qualify(output, tagged_validator, node):
    tagged, runtime = binding(tagged_validator), binding(Path(node).resolve())
    require(tagged['sha256'] == binding(Path(__file__).with_name('ios_runtime_acceptance.py'))['sha256'], 'Tagged v3 changed')
    checker_sources = sources()
    out = Path(output); out.mkdir(mode=0o700, exist_ok=False)
    command = [sys.executable,'-B','-m','unittest','-v',*TESTS]
    env = {k:v for k,v in os.environ.items() if not k.startswith(('NODE_','PYTHON'))}
    env['IRIS_VOIP_TEST_NODE'] = runtime['path']
    with (out/'tests.log').open('xb') as log:
        result = subprocess.run(command, cwd=Path(__file__).parent, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=90)
    for value in [*checker_sources, tagged, runtime]: verify_binding(value)
    save(out/'qualification.json', {'schema':'iris-voip-wake-qualification-v1','passed':result.returncode==0,
        'exit_code':result.returncode,'device_actions':False,'network_requests':0,'native_builds':False,
        'physical_release_acceptance':False,'checker_sources':checker_sources,'tests':list(TESTS),
        'tests_log':binding(out/'tests.log'),'tagged_validator':tagged,'node':runtime,
        'python':binding(Path(sys.executable).resolve()),'command':command})
    return result.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    q = sub.add_parser('qualify')
    q.add_argument('--out',required=True); q.add_argument('--tagged-validator',required=True); q.add_argument('--node',required=True)
    run = sub.add_parser('observe')
    run.add_argument('--input',required=True); run.add_argument('--input-sha256',required=True)
    run.add_argument('--output',required=True); run.add_argument('--execute',action='store_true')
    args = parser.parse_args()
    if args.action == 'qualify': return qualify(args.out,args.tagged_validator,args.node)
    observe({'path':str(Path(args.input).resolve()),'sha256':args.input_sha256},args.output,execute=args.execute)
    return 0


if __name__ == '__main__': raise SystemExit(main())

