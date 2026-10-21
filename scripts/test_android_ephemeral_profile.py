#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from android_ephemeral_profile import (OwnedEphemeralUser, WIZARD_STAGES, foreground_package,
                                       prepare_owned_ephemeral_profile, verify_owned_ephemeral_ui, wizard_control)

HOME = '<hierarchy><node package="com.android.launcher3" /></hierarchy>'
SHADE = '<hierarchy><node package="com.android.systemui" resource-id="com.android.systemui:id/notification_panel" /></hierarchy>'
WIZARD = '<hierarchy><node package="app.grapheneos.setupwizard" text="Welcome to GrapheneOS" /></hierarchy>'
POLICY = 'KeyguardServiceDelegate\n showing=false\n inputRestricted=false\n interactiveState=INTERACTIVE_STATE_AWAKE\n'
PNG = b'\x89PNG\r\n\x1a\nimage'


def wizard_xml(stage):
    name, package, title, label = WIZARD_STAGES[stage]
    root = ET.Element('hierarchy')
    page = ET.SubElement(root, 'node', package=package, text=title)
    if name == 'empty-pin':
        ET.SubElement(page, 'node', package=package, text='PIN must be at least 4 digits, but a 6-digit PIN is recommended for added security')
        ET.SubElement(page, 'node', {'package': package, 'class': 'android.widget.EditText',
                      'resource-id': 'com.android.settings:id/password_entry', 'text': ''})
    elif name == 'confirm-skip-pin':
        ET.SubElement(page, 'node', package=package, text='A PIN is required to set up Fingerprint Unlock.\n\nA PIN protects the phone if it’s lost or stolen.')
        ET.SubElement(page, 'node', package=package, text='Cancel')
    elif name == 'location':
        ET.SubElement(page, 'node', {'package': package, 'class': 'android.widget.Switch',
                      'text': 'Use location', 'checked': 'true', 'clickable': 'true', 'bounds': '[0,0][10,10]'})
    ET.SubElement(page, 'node', {'package': package, 'class': 'android.widget.Button', 'text': label,
                  'clickable': 'true', 'enabled': 'true', 'bounds': '[20,30][40,50]'})
    return ET.tostring(root, encoding='unicode')


class Device:
    def __init__(self, wizard=False):
        self.calls = []
        self.current = '25'
        self.flags = '510'
        self.shade = False
        self.wizard = wizard
        self.stage = 0
        self.complete = False
        self.wizard_pages = [wizard_xml(i) for i in range(len(WIZARD_STAGES))]
        self.home_ui = HOME
        self.shade_ui = SHADE
        self.policy = POLICY

    def __call__(self, *parts, **kwargs):
        self.calls.append(parts)
        args = parts[1:] if parts[0] == 'shell' else parts
        if args == ('pm', 'list', 'users'): return 'UserInfo{0:Owner:c13} running\nUserInfo{25:Test:' + self.flags + '} running\n'
        if args == ('am', 'get-current-user'): return self.current
        if args[:2] == ('pm', 'path'): return 'package:/system/app/stub.apk'
        if args == ('settings', '--user', '25', 'get', 'secure', 'user_setup_complete'):
            return '1' if self.complete else '0'
        if args[:2] == ('input', 'tap'):
            if not self.wizard or args[2:] != ('30', '40'):
                raise AssertionError('Only the actual normal control may be tapped')
            self.stage += 1
            if self.stage == len(WIZARD_STAGES): self.wizard = False; self.complete = True
            return ''
        if args[:2] == ('input', 'keyevent'): return ''
        if args == ('wm', 'dismiss-keyguard'): return ''
        if args[:3] == ('cmd', 'package', 'resolve-activity'): return 'com.android.launcher3/.QuickstepLauncher'
        if args == ('dumpsys', 'window', 'policy'): return self.policy
        if args[:2] == ('uiautomator', 'dump'): return 'UI hierarchy dumped'
        if args[0] == 'cat':
            return self.wizard_pages[self.stage] if self.wizard else (self.shade_ui if self.shade else self.home_ui)
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

    def test_normal_six_control_flow_then_real_home_shade_and_restored_home(self):
        d=Device(wizard=True)
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp)/'proof'
            r=prepare_owned_ephemeral_profile(d,ownership(),out,lambda:PNG)
            self.assertTrue(r['passed']);self.assertTrue(r['shade_collapsed'])
            self.assertEqual([a['label'] for a in r['wizard']['actions']],['Next','Next','Skip','Skip','Skip','Start'])
            self.assertTrue(r['wizard']['observed_user_setup_complete'])
            self.assertEqual(len(list((out/'wizard').glob('*.xml'))),13)
            self.assertEqual(len(list((out/'wizard').glob('*.png'))),13)
            self.assertEqual(r['screens']['home']['foreground_package'],'com.android.launcher3')
            self.assertEqual(r['screens']['shade']['foreground_package'],'com.android.systemui')
            self.assertEqual(r['screens']['restored-home']['foreground_package'],'com.android.launcher3')
        self.assertEqual(len([c for c in d.calls if c[1:3]==('input','tap')]),6)
        self.assertIn('checked="true"',d.wizard_pages[1])
        self.assertFalse(any(any(arg in c for arg in ('device_provisioned','put','disable-user','force-stop','clear','text')) for c in d.calls))
        self.assertFalse(d.shade)

    def test_nonempty_existing_credential_wrong_input_or_unknown_confirmation_is_rejected(self):
        pin=wizard_xml(2)
        for xml,stage in [(pin.replace('text=""','text="1234"'),2),
                          (pin.replace('password_entry','existing_password'),2),
                          (pin.replace('android.widget.EditText','other.PasswordEditText'),2),
                          (pin.replace('Set a PIN','Enter your PIN'),2),
                          (pin.replace('PIN must be at least 4 digits','Confirm your current PIN'),2),
                          (wizard_xml(0).replace('Next','Sign in'),0),
                          (wizard_xml(3).replace('Skip setup for PIN • Fingerprint?','Disable screen lock?'),3)]:
            with self.subTest(stage=stage,xml=xml),self.assertRaises(ValueError):wizard_control(xml,stage)

    def test_duplicate_disabled_wrong_page_or_invalid_control_never_taps(self):
        base=wizard_xml(0);root=ET.fromstring(base)
        root[0].append(ET.fromstring(ET.tostring(root[0][-1])))
        for xml in [ET.tostring(root,encoding='unicode'),base.replace('enabled="true"','enabled="false"'),
                    base.replace('[20,30][40,50]','[40,30][20,50]'),wizard_xml(1),'<hierarchy/>']:
            d=Device(wizard=True);d.wizard_pages[0]=xml
            with tempfile.TemporaryDirectory() as tmp,self.assertRaises(ValueError):
                prepare_owned_ephemeral_profile(d,ownership(),Path(tmp)/'proof',lambda:PNG)
            self.assertFalse(any(c[1:3]==('input','tap') for c in d.calls))

    def test_page_changes_or_user_switch_before_tap_prevents_action(self):
        for change_user in [False,True]:
            d=Device(wizard=True);captures=0
            def screenshot():
                nonlocal captures
                captures+=1
                if captures==1:
                    if change_user:d.current='0'
                    else:d.wizard_pages[0]=wizard_xml(0).replace('[20,30][40,50]','[21,30][41,50]')
                return PNG
            with tempfile.TemporaryDirectory() as tmp,self.assertRaises(ValueError):
                prepare_owned_ephemeral_profile(d,ownership(),Path(tmp)/'proof',screenshot)
            self.assertFalse(any(c[1:3]==('input','tap') for c in d.calls))

    def test_wizard_must_set_its_own_completion_and_failure_proof_is_preserved(self):
        d=Device(wizard=True)
        def adb(*parts,**kw):
            result=d(*parts,**kw)
            return '0' if parts[-1]=='user_setup_complete' else result
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp)/'proof'
            with self.assertRaisesRegex(ValueError,'own user setup'):
                prepare_owned_ephemeral_profile(adb,ownership(),out,lambda:PNG)
            r=json.loads((out/'receipt.json').read_text())
            self.assertFalse(r['passed']);self.assertFalse(r['wizard']['observed_user_setup_complete'])
            self.assertTrue((out/'wizard'/'completed.xml').is_file())
            self.assertFalse((out/'ordinary-ui').exists())

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
