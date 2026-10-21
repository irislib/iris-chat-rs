#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest

from android_ephemeral_profile import OwnedEphemeralUser, foreground_package, prepare_owned_ephemeral_profile, verify_owned_ephemeral_ui

HOME = '<hierarchy><node package="com.android.launcher3" /></hierarchy>'
SHADE = '<hierarchy><node package="com.android.systemui" resource-id="com.android.systemui:id/notification_panel" /></hierarchy>'
WIZARD = '<hierarchy><node package="app.grapheneos.setupwizard" text="Welcome to GrapheneOS" /></hierarchy>'
POLICY = 'KeyguardServiceDelegate\n showing=false\n inputRestricted=false\n interactiveState=INTERACTIVE_STATE_AWAKE\n'


class Device:
    def __init__(self):
        self.calls = []
        self.current = '25'
        self.flags = '510'
        self.shade = False
        self.wizard_disabled = False
        self.home_ui = HOME
        self.shade_ui = SHADE
        self.policy = POLICY

    def __call__(self, *parts, **kwargs):
        self.calls.append(parts)
        args = parts[1:] if parts[0] == 'shell' else parts
        if args == ('pm', 'list', 'users'): return 'UserInfo{0:Owner:c13} running\nUserInfo{25:Test:' + self.flags + '} running\n'
        if args == ('am', 'get-current-user'): return self.current
        if args[:2] == ('pm', 'path'): return 'package:/system/app/stub.apk'
        if args[:2] == ('pm', 'disable-user'):
            self.wizard_disabled = True
            return 'new state: disabled-user'
        if args[:3] == ('pm', 'list', 'packages'): return 'package:' + args[-1] if self.wizard_disabled else ''
        if args[:2] == ('settings', '--user'): return ''
        if args[:2] == ('input', 'keyevent'): return ''
        if args == ('wm', 'dismiss-keyguard'): return ''
        if args[:3] == ('cmd', 'package', 'resolve-activity'): return 'com.android.launcher3/.QuickstepLauncher'
        if args == ('dumpsys', 'window', 'policy'): return self.policy
        if args[:2] == ('uiautomator', 'dump'): return 'UI hierarchy dumped'
        if args[0] == 'cat': return self.shade_ui if self.shade else self.home_ui
        if args[:2] == ('cmd', 'statusbar'):
            self.shade = args[2] == 'expand-notifications'
            return ''
        raise AssertionError('Unexpected device command: ' + repr(parts))


def ownership():
    return OwnedEphemeralUser.from_creation(0, {0, 10}, 'Success: created user id 25')


class ProfileTests(unittest.TestCase):
    def test_cannot_target_primary_preexisting_or_unproven_user(self):
        for original, existing, response in [(10,{0},'Success: created user id 25'),(0,{0,25},'Success: created user id 25'),(0,{0},'Success: created user id 0'),(0,{0},'25')]:
            with self.subTest(response=response), self.assertRaises(ValueError):
                OwnedEphemeralUser.from_creation(original,existing,response)

    def test_non_ephemeral_or_noncurrent_user_has_no_mutation(self):
        for field, value in [('flags','410'),('current','0')]:
            d=Device();setattr(d,field,value)
            with tempfile.TemporaryDirectory() as tmp,self.assertRaises(ValueError):
                prepare_owned_ephemeral_profile(d,ownership(),Path(tmp)/'proof',lambda:b'')
            self.assertTrue(all(c[1:3] in [('pm','list'),('am','get-current-user')] for c in d.calls))

    def test_actual_grapheneos_wizard_disabled_only_in_owned_user(self):
        d=Device()
        with tempfile.TemporaryDirectory() as tmp:
            r=prepare_owned_ephemeral_profile(d,ownership(),Path(tmp)/'proof',lambda:b'\x89PNG\r\n\x1a\nimage')
            self.assertTrue(r['passed']);self.assertTrue(r['shade_collapsed'])
            self.assertIn('app.grapheneos.setupwizard',r['disabled_owned_user_wizards'])
            self.assertEqual(r['screens']['home']['foreground_package'],'com.android.launcher3')
            self.assertEqual(r['screens']['shade']['foreground_package'],'com.android.systemui')
            self.assertEqual(r['screens']['restored-home']['foreground_package'],'com.android.launcher3')
        changes=[c for c in d.calls if c[1:3] in [('settings','--user'),('pm','disable-user')]]
        self.assertTrue(changes)
        for command in changes:self.assertEqual(command[command.index('--user')+1],'25')
        self.assertFalse(any('device_provisioned' in c or 'force-stop' in c or 'clear' in c for c in d.calls))
        self.assertFalse(d.shade)

    def test_absent_wizard_is_not_disabled_and_source_failure_is_preserved(self):
        d=Device();original=d.__call__
        def adb(*parts,**kw):
            if parts[:3]==('shell','pm','path') and parts[-1]=='com.google.android.setupwizard':
                d.calls.append(parts)
                return ''
            return original(*parts,**kw)
        with tempfile.TemporaryDirectory() as tmp:
            r=prepare_owned_ephemeral_profile(adb,ownership(),Path(tmp)/'proof',lambda:b'\x89PNG\r\n\x1a\nimage')
        self.assertEqual(r['disabled_owned_user_wizards'],['app.grapheneos.setupwizard'])
        self.assertFalse(any(c[1:3]==('pm','disable-user') and c[-1]=='com.google.android.setupwizard' for c in d.calls))

    def test_saved_failure_shape_is_rejected_not_mistaken_for_home(self):
        with self.assertRaisesRegex(ValueError,'setup wizard'):foreground_package(WIZARD)
        d=Device();d.home_ui=WIZARD
        with tempfile.TemporaryDirectory() as tmp,self.assertRaisesRegex(ValueError,'setup wizard'):
            verify_owned_ephemeral_ui(d,ownership(),Path(tmp)/'proof',lambda:b'\x89PNG\r\n\x1a\nimage')
        self.assertFalse(d.shade)

    def test_empty_keyguard_wrong_launcher_or_unexpanded_shade_fail(self):
        for field,value in [('home_ui','<hierarchy/>'),('home_ui','<hierarchy><node package="other.app"/></hierarchy>'),('policy',POLICY.replace('showing=false','showing=true')),('shade_ui',HOME)]:
            d=Device();setattr(d,field,value)
            with self.subTest(field=field),tempfile.TemporaryDirectory() as tmp,self.assertRaises(ValueError):
                verify_owned_ephemeral_ui(d,ownership(),Path(tmp)/'proof',lambda:b'\x89PNG\r\n\x1a\nimage')
            self.assertFalse(d.shade)

    def test_current_user_switch_prevents_owner_ui_action(self):
        d=Device();original=d.__call__;count=0
        def adb(*parts,**kw):
            nonlocal count
            if parts==('shell','am','get-current-user'):
                count+=1
                if count==2:d.current='0'
            return original(*parts,**kw)
        with tempfile.TemporaryDirectory() as tmp,self.assertRaises(ValueError):
            verify_owned_ephemeral_ui(adb,ownership(),Path(tmp)/'proof',lambda:b'')
        self.assertFalse(any(c[1:3] in [('input','keyevent'),('cmd','statusbar')] for c in d.calls))

    def test_failed_wizard_snapshot_is_saved_before_profile_cleanup(self):
        d=Device();d.home_ui=WIZARD
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp)/'proof'
            with self.assertRaises(ValueError):
                verify_owned_ephemeral_ui(d,ownership(),out,lambda:b'\x89PNG\r\n\x1a\nimage')
            self.assertEqual((out/'home.xml').read_text(),WIZARD)
            self.assertTrue((out/'home.png').is_file())
            self.assertFalse(json.loads((out/'receipt.json').read_text())['passed'])


if __name__=='__main__':unittest.main()
