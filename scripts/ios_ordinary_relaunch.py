#!/usr/bin/env python3
"""Private evidence checks for one ordinary foreground replacement launch.

This module does not install, sign, publish, or contact a device on import. An
operator supplies the qualified provenance record and reserved capture/transport
callbacks. Qualification of this source is not physical release acceptance.
"""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import time
from urllib.parse import urlsplit

import ios_runtime_acceptance as contract
import ios_relaunch_lifecycle as lifecycle


class EvidenceError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise EvidenceError(message)


def exact(value, keys):
    require(isinstance(value, dict) and set(value) == set(keys), "Unexpected evidence fields")


def finite(value):
    require(type(value) in (int, float) and math.isfinite(value), "Invalid evidence clock")
    return value


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def binding(path):
    path = Path(path)
    require(path.is_file() and not path.is_symlink(), "Missing regular evidence file")
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return {"path": str(path.resolve()), "sha256": h.hexdigest()}


def verify_binding(value):
    exact(value, {"path", "sha256"})
    require(isinstance(value['path'], str) and Path(value['path']).is_absolute(), "Evidence path must be absolute")
    contract.digest(value['sha256'])
    require(binding(value['path']) == value, "Evidence bytes changed")
    return Path(value['path'])


def read_bound(value, limit=1024 * 1024):
    path = verify_binding(value)
    with path.open('rb') as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit and sha256(data) == value['sha256'], "Evidence read changed or exceeds bound")
    return json.loads(data.decode('utf-8'), object_pairs_hook=contract.unique_object,
                      parse_constant=lambda _: (_ for _ in ()).throw(EvidenceError('Nonfinite JSON')))


PROVENANCE_INPUTS = {'authorization', 'release_acceptance', 'preparation', 'installation',
                     'original_signing', 'profiles', 'provenance_qualification', 'official_ipa', 'manifest'}
PROVENANCE_DIGESTS = contract.ADHOC_DIGESTS - {'normal_launch_sha256', 'qualification_sha256'}
CODE_KEYS = {'uuid', 'loadCommandsSHA256', 'unsignedContentSHA256'}
TARGET_KEYS = {'device', 'coredevice_id', 'bundle_id', 'installed_url', 'previous_build', 'installation_finished_utc'}
HOST_CLOCK_TOLERANCE_SECONDS = 0.25


def validate_armed(armed, distribution):
    exact(armed, {'armed', 'nonce', 'device', 'bundle_id', 'build'})
    require(armed['armed'] is True and isinstance(armed['nonce'], str) and len(armed['nonce']) >= 16
            and armed['device'] == distribution['device']
            and armed['bundle_id'] == distribution['bundle_id']
            and armed['build'] == distribution['candidate']['apple_build'], 'Wrong prearmed target')


def validate_trigger(armed, proof, result):
    exact(result, {'nonce', 'build', 'pid', 'sent_count', 'acknowledged'})
    require(result['nonce'] == armed['nonce'] and result['build'] == armed['build']
            and result['pid'] == proof['pid'] and type(result['sent_count']) is int
            and result['sent_count'] == 1 and result['acknowledged'] is True,
            'Trigger failed or acknowledged another attempt')


def validate_distribution(config_binding, candidate):
    """Bind an upstream qualified signing/install proof; never fabricate it.

    The private input is a normalized output of the separately qualified
    provisioning/install checks, reviewed by the release owner. Raw source
    receipts remain hash-bound. This launch checker does not replace codesign,
    profile validation, in-place installation or physical preserved-state checks.
    """
    contract.validate_candidate(candidate)
    config = read_bound(config_binding)
    exact(config, {'schema', 'candidate', 'production_adhoc', 'inputs', 'code_bindings'} | TARGET_KEYS)
    require(config['schema'] == 'iris-ordinary-relaunch-input-v1', 'Wrong private input schema')
    require(config['candidate'] == candidate, 'Wrong candidate')
    for key in ('device', 'coredevice_id', 'bundle_id'):
        require(isinstance(config[key], str) and config[key] and not any(c.isspace() for c in config[key]), 'Invalid target')
    url = urlsplit(config['installed_url'])
    require(url.scheme == 'file' and not url.netloc and not url.query and not url.fragment
            and url.path.endswith('.app/'), 'Invalid installed app URL')
    require(isinstance(config['previous_build'], str) and config['previous_build'].isdigit()
            and 0 < int(config['previous_build']) < int(candidate['apple_build']), 'Not an in-place upgrade')
    finite(config['installation_finished_utc'])
    exact(config['inputs'], PROVENANCE_INPUTS)
    for value in config['inputs'].values():
        verify_binding(value)
    require(config['inputs']['official_ipa']['sha256'] == candidate['ipa_sha256']
            and config['inputs']['manifest']['sha256'] == candidate['manifest_sha256'], 'Wrong official artifacts')
    proof = config['production_adhoc']
    exact(proof, PROVENANCE_DIGESTS | set(contract.ADHOC_POLICY) |
          {'ipa_sha256', 'bundle_instruction_sha256', 'testflight_upgrade_claimed', 'installed_binary_extraction_claimed'})
    for key, expected in contract.ADHOC_POLICY.items():
        require(proof[key] == expected, 'Unqualified distribution policy')
    for key in PROVENANCE_DIGESTS:
        require(proof[key] == config['inputs'][key.removesuffix('_sha256')]['sha256'], 'Provenance digest mismatch')
    require(proof['ipa_sha256'] == candidate['ipa_sha256'], 'Wrong IPA proof')
    require(proof['testflight_upgrade_claimed'] is False and proof['installed_binary_extraction_claimed'] is False,
            'Unsupported distribution claim')
    roles = {'app', 'notification-service', 'share-extension'}
    exact(config['code_bindings'], roles)
    exact(proof['bundle_instruction_sha256'], roles)
    for role, pair in config['code_bindings'].items():
        exact(pair, {'original', 'resigned'})
        exact(pair['original'], CODE_KEYS)
        require(pair['original'] == pair['resigned'], 'Executable instructions or metadata changed')
        require(isinstance(pair['original']['uuid'], str) and pair['original']['uuid'], 'Missing executable UUID')
        for field in ('loadCommandsSHA256', 'unsignedContentSHA256'):
            contract.digest(pair['original'][field])
        encoded = json.dumps(pair['original'], sort_keys=True, separators=(',', ':')).encode()
        require(proof['bundle_instruction_sha256'][role] == sha256(encoded), 'Code proof digest mismatch')
    qualification = read_bound(config['inputs']['provenance_qualification'])
    exact(qualification, {'schema', 'candidate', 'target', 'inputs', 'production_adhoc', 'code_bindings', 'checker_sources', 'review'})
    require(qualification['schema'] == 'iris-qualified-adhoc-provenance-v1'
            and qualification['candidate'] == candidate, 'Wrong provenance qualification')
    require(qualification['target'] == {k: config[k] for k in TARGET_KEYS}, 'Qualification target changed')
    require(qualification['inputs'] == {k: v for k, v in config['inputs'].items() if k != 'provenance_qualification'}
            and qualification['production_adhoc'] == proof
            and qualification['code_bindings'] == config['code_bindings'], 'Qualification does not bind this proof')
    require(isinstance(qualification['checker_sources'], list) and qualification['checker_sources'], 'No qualified provenance checker')
    for value in qualification['checker_sources']:
        verify_binding(value)
    # A review is retained evidence, not an inferred approval from a digest.
    review = read_bound(qualification['review'])
    exact(review, {'schema', 'decision', 'candidate', 'target', 'inputs', 'checker_sources', 'code_bindings'})
    require(review['schema'] == 'iris-adhoc-provenance-review-v1' and review['decision'] == 'accepted',
            'Provenance has not been reviewed')
    for key in ('candidate', 'target', 'inputs', 'checker_sources', 'code_bindings'):
        require(review[key] == qualification[key], 'Provenance review is stale')
    return config


def process(row):
    exact(row, {'pid', 'executable'})
    require(type(row['pid']) is int and 0 < row['pid'] < 2**31, 'Invalid PID')
    require(isinstance(row['executable'], str) and row['executable'], 'Missing executable')
    return row


def validate_snapshot(snapshot, executable, *, now, foreground=False):
    exact(snapshot, {'observed_utc', 'processes', 'foreground_pid'})
    require(0 <= finite(now) - finite(snapshot['observed_utc']) <= 8, 'Stale process inventory')
    require(isinstance(snapshot['processes'], list) and len(snapshot['processes']) == 1, 'Expected one target process')
    row = process(snapshot['processes'][0])
    require(row['executable'] == executable, 'Wrong executable inventory')
    if foreground:
        require(snapshot['foreground_pid'] == row['pid'], 'Replacement requires actual foreground process')
    return row


def launch_command(bundle_id, *, replacement):
    return ['device', 'process', 'launch', *(['--terminate-existing'] if replacement else []), bundle_id]


def validate_launch_result(distribution, installed, result, command, started, returned, validated,
                           *, executable, before=None, resume_of=None):
    """Check returned target/new PID/options in memory before one control action.

    This is deliberately insufficient for final acceptance: the separate actual
    postlaunch inventory and every lifecycle check remain mandatory.
    """
    require((before is None) != (resume_of is None), 'Choose replacement or same-process resume')
    replacement = before is not None
    require(command == launch_command(distribution['bundle_id'], replacement=replacement), 'Nonordinary launch command')
    require(installed.get('bundleIdentifier') == distribution['bundle_id']
            and installed.get('bundleVersion') == distribution['candidate']['apple_build']
            and installed.get('url') == distribution['installed_url'], 'Installed candidate mismatch')
    require(executable.startswith(distribution['installed_url']) and '/' not in executable[len(distribution['installed_url']):]
            and executable != distribution['installed_url'], 'Executable outside installed app')
    require(distribution['installation_finished_utc'] <= finite(started) <= finite(returned) <= finite(validated)
            and validated - returned <= 8, 'Stale or reordered launch')
    require(result.get('deviceIdentifier') == distribution['coredevice_id'], 'Wrong launch device')
    options = result.get('launchOptions')
    exact(options, {'activatedWhenStarted', 'arguments', 'environmentVariables', 'platformSpecificOptions',
                    'startStopped', 'terminateExistingInstances', 'user'})
    require(options['activatedWhenStarted'] is True and options['startStopped'] is False
            and options['terminateExistingInstances'] is replacement and options['arguments'] == []
            and options['platformSpecificOptions'] == {} and options['user'] == {'active': True}
            and options['environmentVariables'] in ({}, {'TERM': 'dumb'}), 'Nonordinary launch options')
    launched = result.get('process')
    require(isinstance(launched, dict), 'No launch result process')
    pid, audit = launched.get('processIdentifier'), launched.get('auditToken')
    process({'pid': pid, 'executable': launched.get('executable')})
    require(launched['executable'] == executable and isinstance(audit, list) and len(audit) == 8
            and all(type(n) is int and 0 <= n <= 2**32 - 1 for n in audit) and audit[5] == pid,
            'Invalid launch process identity')
    if replacement:
        old = validate_snapshot(before, executable, now=started, foreground=True)
        require(old['pid'] != pid, 'Replacement did not create a fresh process')
    else:
        require(pid == resume_of['pid'] and audit == resume_of['audit_token']
                and executable == resume_of['executable'], 'Resume changed process identity')
    return {'route': 'accepted-adhoc-normal-launch', 'candidate': distribution['candidate'],
            'pid': pid, 'executable': executable, 'audit_token': audit,
            'replacement': replacement, 'previous_pid': before['processes'][0]['pid'] if replacement else None,
            'command': command, 'launch_options': options, 'started_utc': started,
            'returned_utc': returned, 'validated_utc': validated, 'debugger_free': True,
            'inventory_verified': False}


def validate_launch(distribution, installed, result, snapshot, command, started, returned, validated,
                    *, executable, before=None, resume_of=None):
    """Finish the ordinary-launch proof using actual postlaunch inventory."""
    proof = validate_launch_result(distribution, installed, result, command, started, returned, validated,
                                   executable=executable, before=before, resume_of=resume_of)
    current = validate_snapshot(snapshot, executable, now=validated)
    require(snapshot['observed_utc'] >= returned and current['pid'] == proof['pid'], 'Launch/inventory mismatch')
    return {**proof, 'inventory_verified': True}


class OrdinaryForegroundRelaunch:
    """One attempt only, including uncertain failures; caller owns reservation.

    Callbacks must capture real process/OS state and actual CoreDevice results.
    `prearm` returns a target-bound acknowledgement without touching the app.
    `capture_prefixes` returns the ready, complete raw-log boundaries before launch.
    The trigger is sent once after fresh launch validation; its acknowledgement
    never establishes real background entry or restoration overlap.
    """
    def __init__(self, distribution_binding, candidate, installed, executable, *, clock=time.time, monotonic=time.monotonic):
        self.binding = distribution_binding
        self.candidate = candidate
        self.distribution = validate_distribution(distribution_binding, candidate)
        self.installed, self.executable, self.clock, self.monotonic = installed, executable, clock, monotonic
        self.attempted = False
        self.record = {'schema': 'iris-ordinary-foreground-relaunch-v1', 'completed': False,
                       'physical_acceptance': False, 'launch_count': 0, 'trigger_count': 0,
                       'automatic_retry': False}

    def run_once(self, *, snapshot, prearm, capture_prefixes, launch, trigger):
        require(not self.attempted, 'An attempted launch cannot be retried')
        self.attempted = True
        try:
            require(validate_distribution(self.binding, self.candidate) == self.distribution, 'Distribution changed')
            armed = prearm()
            validate_armed(armed, self.distribution)
            before = snapshot()
            validate_snapshot(before, self.executable, now=self.clock(), foreground=True)
            captures = capture_prefixes()
            exact(captures, {'startup_text', 'lifecycle_text'})
            prefixes = lifecycle.capture_prefixes(**captures, device=self.distribution['device'])
            self.record.update(armed=armed, before=before, initial_prefixes=prefixes)
            command = launch_command(self.distribution['bundle_id'], replacement=True)
            started = self.clock()
            validate_snapshot(before, self.executable, now=started, foreground=True)
            self.record['launch_count'] = 1
            self.record.update(command=command, started_utc=started, started_monotonic=self.monotonic(),
                               installed=self.installed, executable=self.executable)
            result = launch(command)
            returned = self.clock()
            self.record.update(result=result, returned_utc=returned, returned_monotonic=self.monotonic())
            immediate = validate_launch_result(self.distribution, self.installed, result, command,
                started, returned, self.clock(), executable=self.executable, before=before)
            self.record['launch_result_proof'] = immediate
            self.record['trigger_count'] = 1
            trigger_result = trigger(armed, immediate)
            self.record['trigger_result'] = trigger_result
            validate_trigger(armed, immediate, trigger_result)
            after = snapshot()
            proof = validate_launch(self.distribution, self.installed, result, after, command,
                started, returned, self.clock(), executable=self.executable, before=before)
            self.record.update(after=after, launch_proof=proof)
            require(validate_distribution(self.binding, self.candidate) == self.distribution, 'Distribution changed')
            self.record['completed'] = True
            return self.record
        except Exception as error:
            self.record['failure'] = str(error)
            raise


def evaluate_runtime(distribution_binding, candidate, attempt, resume, capture):
    """Re-evaluate raw launch/resume/capture evidence, not recorded pass flags."""
    distribution = validate_distribution(distribution_binding, candidate)
    require(attempt.get('schema') == 'iris-ordinary-foreground-relaunch-v1'
            and type(attempt.get('launch_count')) is int and attempt['launch_count'] == 1
            and type(attempt.get('trigger_count')) is int and attempt['trigger_count'] == 1
            and attempt.get('automatic_retry') is False and attempt.get('completed') is True,
            'Incomplete one-shot attempt')
    require('failure' not in attempt, 'A preserved failure cannot become acceptance')
    immediate = validate_launch_result(distribution, attempt['installed'], attempt['result'], attempt['command'],
        attempt['started_utc'], attempt['returned_utc'], attempt['launch_result_proof']['validated_utc'],
        executable=attempt['executable'], before=attempt['before'])
    require(immediate == attempt['launch_result_proof'], 'Immediate launch result proof changed')
    proof = validate_launch(distribution, attempt['installed'], attempt['result'], attempt['after'],
                            attempt['command'], attempt['started_utc'], attempt['returned_utc'],
                            attempt['launch_proof']['validated_utc'], executable=attempt['executable'],
                            before=attempt['before'])
    require(attempt['launch_proof'] == proof, 'Recorded launch proof changed')
    validate_armed(attempt['armed'], distribution)
    validate_trigger(attempt['armed'], proof, attempt['trigger_result'])
    exact(resume, {'result', 'snapshot', 'command', 'started_utc', 'returned_utc', 'validated_utc',
                   'started_monotonic', 'returned_monotonic'})
    require(resume['started_utc'] > proof['returned_utc'], 'Resume preceded initial launch')
    resume_proof = validate_launch(distribution, attempt['installed'], resume['result'], resume['snapshot'],
                                   resume['command'], resume['started_utc'], resume['returned_utc'],
                                   resume['validated_utc'], executable=proof['executable'], resume_of=proof)
    exact(capture, {'startup_text', 'lifecycle_text', 'process_name', 'samples', 'measurement_cutoff', 'closure_binding'})
    closure = read_bound(capture['closure_binding'])
    exact(closure, {'schema', 'candidate', 'device', 'closed', 'failures', 'groups', 'measurement_cutoff_sha256'})
    require(closure['schema'] == 'iris-relaunch-adapter-closure-v1' and closure['closed'] is True
            and closure['failures'] == [] and closure['candidate'] == candidate
            and closure['device'] == distribution['device'] and isinstance(closure['groups'], list)
            and len(closure['groups']) >= 3
            and all(type(group) is int and group > 0 for group in closure['groups']),
            'Capture process groups did not close for this candidate')
    require(closure['measurement_cutoff_sha256'] == lifecycle.evidence_sha256(capture['measurement_cutoff']),
            'Closure does not bind the exact live measurement cutoff')
    require(capture['process_name'] == urlsplit(proof['executable']).path.rsplit('/', 1)[-1],
            'Capture process name differs from launched executable')
    evidence = lifecycle.evaluate_early_background(**{k: v for k, v in capture.items() if k != 'closure_binding'}, bundle_id=distribution['bundle_id'],
        pid=proof['pid'], executable=proof['executable'], device=distribution['device'],
        initial_prefixes=attempt['initial_prefixes'], old_pid=proof['previous_pid'])
    require(finite(attempt['started_monotonic']) <= finite(attempt['returned_monotonic'])
            <= capture['samples'][0]['observed_monotonic'], 'Capture preceded launch return')
    require(evidence['suspended_last_monotonic'] <= finite(resume['started_monotonic'])
            <= finite(resume['returned_monotonic']) <= evidence['resumed_started_monotonic'],
            'Resume command was not between suspension and observed resumption')
    offset = finite(attempt['started_utc']) - finite(attempt['started_monotonic'])
    for record, boundary in ((attempt, 'returned'), (resume, 'started'), (resume, 'returned')):
        require(abs(finite(record[boundary + '_utc']) - finite(record[boundary + '_monotonic']) - offset)
                <= HOST_CLOCK_TOLERANCE_SECONDS, 'Host clock discontinuity or stale resume evidence')
    require(validate_distribution(distribution_binding, candidate) == distribution, 'Distribution changed during evaluation')
    return {'schema': 'iris-ordinary-relaunch-runtime-v1', 'candidate': distribution['candidate'],
            'launch_proof': proof, 'resume_proof': resume_proof, 'lifecycle': evidence,
            'physical_release_acceptance': False}


def compose_adhoc_proof(distribution_binding, candidate, *, normal_launch_binding, qualification_binding):
    """Only compose after fresh source qualification and real launch validation.

    This does not assert lifecycle, CPU, crash or preserved-state acceptance.
    Those independent candidate-bound fields remain mandatory in tagged v3.
    """
    distribution = validate_distribution(distribution_binding, candidate)
    launch = read_bound(normal_launch_binding, limit=20 * 1024 * 1024)
    exact(launch, {'schema', 'attempt', 'resume', 'capture'})
    require(launch['schema'] == 'iris-ordinary-relaunch-capture-v1', 'Missing original launch/capture evidence')
    evaluate_runtime(distribution_binding, candidate, launch['attempt'], launch['resume'], launch['capture'])
    qualification = read_bound(qualification_binding)
    require(qualification.get('schema') == 'iris-ordinary-relaunch-qualification-v1'
            and qualification.get('synthetic_only') is True and qualification.get('exit_code') == 0
            and qualification.get('sources_unchanged') is True and qualification.get('device_actions') is False
            and qualification.get('native_builds') is False and qualification.get('release_acceptance') is False,
            'Missing independent helper qualification')
    require(qualification.get('provenance_qualification') == distribution['inputs']['provenance_qualification'],
            'Qualification does not bind upstream provenance')
    require(isinstance(qualification.get('sources'), list) and qualification['sources'], 'No source qualification')
    for source in qualification['sources']:
        verify_binding(source)
    from qualify_ios_ordinary_relaunch import SOURCES, TESTS
    require(qualification['sources'] == [binding(Path(__file__).with_name(name)) for name in SOURCES]
            and qualification.get('tests') == list(TESTS), 'Wrong qualified helper/test sources')
    require(verify_binding(qualification['tagged_validator']).read_bytes() == Path(contract.__file__).read_bytes(),
            'Tagged acceptance validator changed')
    verify_binding(qualification['tests_log'])
    require(validate_distribution(distribution_binding, candidate) == distribution, 'Distribution changed before proof composition')
    result = {**distribution['production_adhoc'], 'normal_launch_sha256': normal_launch_binding['sha256'],
              'qualification_sha256': qualification_binding['sha256']}
    contract.validate_adhoc_proof(result, distribution['candidate'])
    return result
