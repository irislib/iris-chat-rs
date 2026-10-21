#!/usr/bin/env python3
"""Configure only fresh-profile Play services through ordinary visible Settings.

GrapheneOS documents this provider prerequisite for background push. Iris must
remain subject to Doze. A physical Settings page still needs XML/PNG qualification;
synthetic host tests alone do not establish that it matches the installed OS.
"""
import hashlib
import json
from pathlib import Path
import re
import time
import xml.etree.ElementTree as ET

GMS = 'com.google.android.gms'
SETTINGS = 'com.android.settings'
OPS = ('RUN_ANY_IN_BACKGROUND', 'RUN_IN_BACKGROUND')


def require_gms_absent(packages_by_user):
    if not packages_by_user or 0 not in packages_by_user:
        raise ValueError('Every preexisting user, including the original user, must be checked')
    if any(type(user) is not int or user < 0 or value.strip()
           for user, value in packages_by_user.items()):
        raise ValueError('Play services already exists in a preexisting user or its baseline is unknown')


def parse_power_allowlist(text):
    rows = []
    for line in text.splitlines():
        match = re.fullmatch(r'(system-excidle|system|user),([A-Za-z][\w.]*),(\d+)', line.strip())
        if match is None:
            raise ValueError('Unknown device-idle allowlist format')
        rows.append((match.group(1), match.group(2), int(match.group(3))))
    if not rows or len(rows) != len(set(rows)):
        raise ValueError('Missing or duplicate device-idle allowlist')
    return frozenset(rows)


def verify_power_allowlist_delta(before, after, owned_uid, enabled):
    added, removed = after - before, before - after
    if not enabled:
        if added or removed:
            raise ValueError('Original device-idle allowlist was not restored exactly')
        return
    if (removed or len(added) != 1 or next(iter(added)) not in
            {('user', GMS, owned_uid), ('user', GMS, owned_uid % 100000)}):
        raise ValueError('Only the owned Play services battery exception may be added')


def effective_background_allowed(text, op):
    """Accept only observed effective allow, never an unreported default."""
    if op not in OPS:
        raise ValueError('Unknown background operation')
    value = text.strip()
    if value == 'No operations.\nDefault mode: allow':
        return True
    if re.fullmatch(r'(?:Uid mode: )?' + re.escape(op) + r': allow(?:; time=[^\n]+)?', value):
        return True
    raise ValueError('Effective background execution is denied or unknown')


def empty_uninstalled_appops(exit_code, stdout, stderr, user):
    """Use only after independently proving GMS absent from every original user.

    PackageManager can retain or remove a global code/UID registration during
    ephemeral-user teardown. Neither empty response grants an original user an
    operation. Any nonempty mode, including an ignore, requires more teardown
    time or explicit diagnosis; it must never be silently discarded.
    """
    if type(user) is not int or user < 0:
        raise ValueError('Exact original user required')
    if exit_code == 0 and stdout.strip() == 'No operations.' and not stderr:
        return True
    if (exit_code == 255 and not stdout
            and stderr.strip() == f'Error: No UID for {GMS} in user {user}'):
        return True
    raise ValueError('Original-user AppOps is nonempty or unknown')


def _bounds(node):
    match = re.fullmatch(r'\[(\d+),(\d+)\]\[(\d+),(\d+)\]', node.get('bounds', ''))
    if match is None:
        raise ValueError('Missing visible Settings control bounds')
    x1, y1, x2, y2 = map(int, match.groups())
    if x2 <= x1 or y2 <= y1:
        raise ValueError('Empty visible Settings control bounds')
    return x1, y1, x2, y2


def _tap_bounds(node):
    x1, y1, x2, y2 = _bounds(node)
    return ((x1 + x2) // 2, (y1 + y2) // 2)


def battery_setting_control(xml, target):
    """Return only the known navigation row or an explicit battery radio option."""
    if target not in ('Unrestricted', 'Optimized') or not xml.strip() or len(xml) > 1024 * 1024:
        raise ValueError('Unknown battery setting or missing Settings hierarchy')
    root = ET.fromstring(xml)
    nodes = list(root.iter('node'))
    labels = [n.get('text', '') for n in nodes]
    headings = labels + [n.get('content-desc', '') for n in nodes]
    if (not nodes or nodes[0].get('package') != SETTINGS
            or any(n.get('package') not in (None, '', SETTINGS) for n in nodes)
            or labels.count('Google Play services') != 1
            or not any(t in headings for t in ('App battery usage', 'Allow background usage'))
            or any(n.get('class', '').endswith('EditText') for n in nodes)):
        raise ValueError('Not the known Play services battery Settings page')
    parents = {child: parent for parent in root.iter() for child in parent}

    def row(label):
        matches = [n for n in nodes if n.get('text') == label]
        if len(matches) != 1:
            raise ValueError('Missing or ambiguous battery option')
        text = matches[0]
        candidate = text
        while candidate is not None:
            if candidate.get('clickable') == 'true' and candidate.get('enabled') == 'true':
                if candidate.get('package') != SETTINGS:
                    break
                return candidate, _tap_bounds(text)
            candidate = parents.get(candidate)
        raise ValueError('Battery option is not an enabled ordinary Settings control')

    has_radios = 'Unrestricted' in labels or 'Optimized' in labels
    if has_radios:
        checked = {}
        for label in ('Optimized', 'Unrestricted'):
            item, _ = row(label)
            radios = [n for n in item.iter('node') if n.get('class') == 'android.widget.RadioButton']
            if len(radios) != 1 or radios[0].get('checked') not in ('true', 'false'):
                raise ValueError('Unknown battery radio state')
            checked[label] = radios[0].get('checked') == 'true'
        if sum(checked.values()) != 1:
            raise ValueError('Ambiguous battery optimization state')
        _, tap = row(target)
        return {'label': target, 'tap': tap, 'changes_setting': True,
                'already_selected': checked[target]}
    item, tap = row('Allow background usage')
    switches = [n for n in item.iter('node') if n.get('class') == 'android.widget.Switch']
    if (len(switches) != 1 or switches[0].get('checked') != 'true'
            or switches[0].get('enabled') != 'true' or switches[0].get('clickable') != 'true'
            or switches[0].get('resource-id') != 'com.android.settings:id/switchWidget'):
        raise ValueError('Known separate enabled background switch is unavailable')
    label = next(n for n in item.iter('node') if n.get('text') == 'Allow background usage')
    x1, y1, x2, y2 = _bounds(label)
    sx1, sy1, sx2, sy2 = _bounds(switches[0])
    if x2 >= sx1 or max(y1, sy1) >= min(y2, sy2):
        raise ValueError('Navigation label must be visibly separate and left of the battery switch')
    return {'label': 'Allow background usage', 'tap': tap,
            'changes_setting': False, 'already_selected': False}


class OwnedPlayServicesBattery:
    """The caller must restore this session before removing its owned user.

    User deletion discards that user's new appops. The package allowlist is
    compared independently because Android versions may share it across users.
    No existing user may have Play services installed when this helper is used.
    """
    def __init__(self, adb, ownership, output, capture_png):
        self.adb, self.ownership, self.capture_png = adb, ownership, capture_png
        self.output = Path(output)
        self.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        self.report = {'passed': False, 'restored': False, 'user': ownership.user,
                       'creation_response_sha256': ownership.creation_response_sha256,
                       'package': GMS, 'actions': [], 'screens': {},
                       'physical_page_qualification_required': True}
        self.before = None
        self.sequence = 0

    def _shell(self, *parts):
        self.ownership.verify(self.adb)
        return self.adb('shell', *parts)

    def _save(self):
        (self.output / 'receipt.json').write_text(json.dumps(self.report, indent=2) + '\n')

    def _snapshot(self):
        raw = self._shell('cmd', 'deviceidle', 'whitelist')
        rows = parse_power_allowlist(raw)
        uid_text = self._shell('pm', 'list', 'packages', '--user', str(self.ownership.user), '-U', GMS)
        match = re.fullmatch(r'package:com\.google\.android\.gms uid:(\d+)', uid_text.strip())
        if match is None or int(match.group(1)) // 100000 != self.ownership.user:
            raise ValueError('Exact owned-profile Play services UID is unavailable')
        appops = {op: self._shell('cmd', 'appops', 'get', '--user', str(self.ownership.user), GMS, op)
                  for op in OPS}
        return {'allowlist': sorted(rows), 'allowlist_raw': raw,
                'uid': int(match.group(1)), 'appops': appops}

    def _capture(self, name):
        name = f'{self.sequence:02d}-{name}'
        self.sequence += 1
        remote = f'/sdcard/iris-owned-play-{self.ownership.user}.xml'
        self._shell('uiautomator', 'dump', remote)
        xml = self._shell('cat', remote)
        (self.output / (name + '.xml')).write_text(xml)
        self.ownership.verify(self.adb)
        png = self.capture_png()
        if not isinstance(png, bytes) or not png.startswith(b'\x89PNG\r\n\x1a\n'):
            raise ValueError('Actual Play services Settings screenshot is unavailable')
        (self.output / (name + '.png')).write_bytes(png)
        self.report['screens'][name] = {'xml_sha256': hashlib.sha256(xml.encode()).hexdigest(),
                                        'png_sha256': hashlib.sha256(png).hexdigest()}
        self._save()
        return xml

    def _set_mode(self, target):
        self._shell('input', 'keyevent', 'WAKEUP')
        self._shell('wm', 'dismiss-keyguard')
        self._shell('am', 'start', '-W', '--user', str(self.ownership.user), '-a',
                    'android.settings.VIEW_ADVANCED_POWER_USAGE_DETAIL', '-d', 'package:' + GMS)
        for _ in range(3):
            control = battery_setting_control(self._capture(target.lower()), target)
            if control['already_selected']:
                return
            # Reclassify the actual page after screenshot collection, before acting.
            if battery_setting_control(self._capture('before-tap'), target) != control:
                raise ValueError('Battery control changed before its action')
            self.report['actions'].append({**control, 'target_mode': target})
            self._save()
            self._shell('input', 'tap', *map(str, control['tap']))
            time.sleep(0.25)
        raise ValueError('Known Play services battery flow exceeded its finite action bound')

    def configure(self):
        try:
            packages = {user: self._shell('pm', 'list', 'packages', '--user', str(user), GMS)
                        for user in sorted(self.ownership.preexisting_users)}
            require_gms_absent(packages)
            self.report['preexisting_users_gms_absent'] = sorted(packages)
            self.before = self._snapshot()
            self.report['before'] = self.before
            if any(row[1] == GMS or row[1].startswith('to.iris.chat')
                   for row in self.before['allowlist']):
                raise ValueError('GMS or Iris already has an exception; preserve the original baseline')
            self._save()
            self._set_mode('Unrestricted')
            after = self._snapshot()
            self.report['after'] = after
            verify_power_allowlist_delta(frozenset(map(tuple, self.before['allowlist'])),
                                        frozenset(map(tuple, after['allowlist'])), after['uid'], True)
            for op in OPS:
                effective_background_allowed(after['appops'][op], op)
            self.report['passed'] = True
            return self.report
        finally:
            self._save()

    def restore(self):
        try:
            if self.before is None:
                self.report['restored'] = True
                return
            current = self._snapshot()
            before_rows = frozenset(map(tuple, self.before['allowlist']))
            current_rows = frozenset(map(tuple, current['allowlist']))
            if current_rows != before_rows:
                verify_power_allowlist_delta(before_rows, current_rows, current['uid'], True)
                self._set_mode('Optimized')
            after = self._snapshot()
            self.report['restoration_snapshot'] = after
            verify_power_allowlist_delta(before_rows, frozenset(map(tuple, after['allowlist'])), after['uid'], False)
            self.report['restored'] = True
        finally:
            self._save()
