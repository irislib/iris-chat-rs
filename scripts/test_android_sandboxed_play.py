#!/usr/bin/env python3
"""Host checks; synthetic Settings trees do not qualify a physical page."""
import unittest
from android_sandboxed_play import (GMS, battery_setting_control, parse_power_allowlist,
                                     verify_power_allowlist_delta, require_gms_absent, effective_background_allowed, empty_uninstalled_appops)


def page(radios=False, selected='Optimized', app='Google Play services'):
    head = '<hierarchy><node package="com.android.settings"><node text="App battery usage" package="com.android.settings"/>'
    head += f'<node text="{app}" package="com.android.settings"/>'
    labels = ['Optimized', 'Unrestricted'] if radios else ['Allow background usage']
    for label in labels:
        head += '<node package="com.android.settings" clickable="true" enabled="true" bounds="[0,100][600,200]">'
        head += f'<node package="com.android.settings" text="{label}" bounds="[20,120][300,160]"/>'
        if radios:
            checked = str(label == selected).lower()
            head += f'<node package="com.android.settings" class="android.widget.RadioButton" checked="{checked}"/>'
        if not radios:
            head += '<node package="com.android.settings" class="android.widget.Switch" resource-id="com.android.settings:id/switchWidget" checked="true" clickable="true" enabled="true" bounds="[400,120][500,160]"/>'
        head += '</node>'
    return head + '</node></hierarchy>'


BASE = 'system,android,1000\nsystem,app.grapheneos.gmscompat,10142\n'


class PlayBatteryTests(unittest.TestCase):
    def test_normal_navigation_and_radio_selection_are_distinct(self):
        c = battery_setting_control(page(), 'Unrestricted')
        self.assertEqual(c['label'], 'Allow background usage')
        self.assertFalse(c['changes_setting'])
        c = battery_setting_control(page(True), 'Unrestricted')
        self.assertEqual(c['label'], 'Unrestricted')
        self.assertTrue(c['changes_setting'])
        self.assertFalse(c['already_selected'])
        self.assertTrue(battery_setting_control(page(True, 'Unrestricted'), 'Unrestricted')['already_selected'])

    def test_restoration_selects_only_optimized(self):
        c = battery_setting_control(page(True, 'Unrestricted'), 'Optimized')
        self.assertEqual(c['label'], 'Optimized')
        self.assertFalse(c['already_selected'])
        for target in ['Restricted', 'Allow background usage', 'com.android.vending']:
            with self.assertRaises(ValueError): battery_setting_control(page(True), target)

    def test_wrong_package_app_or_unknown_page_rejected(self):
        for xml in [page(True, app='Iris'), page(True).replace('com.android.settings','other.settings'),
                    page(True).replace('App battery usage','Device admin'), '<hierarchy/>',
                    page(True).replace('</hierarchy>', '<node class="android.widget.EditText"/></hierarchy>')]:
            with self.subTest(xml=xml), self.assertRaises(ValueError): battery_setting_control(xml, 'Unrestricted')

    def test_ambiguous_disabled_or_unknown_radio_state_rejected(self):
        good=page(True)
        for xml in [good.replace('text="Optimized"','text="Unrestricted"'),
                    good.replace('enabled="true"','enabled="false"'),
                    good.replace('checked="false"','checked="true"'),
                    good.replace('android.widget.RadioButton','android.widget.Switch'),
                    good.replace('[20,120][300,160]','[300,120][20,160]')]:
            with self.subTest(xml=xml), self.assertRaises(ValueError): battery_setting_control(xml, 'Unrestricted')

    def test_only_gms_owned_uid_or_appid_allowlist_addition_is_accepted(self):
        before=parse_power_allowlist(BASE)
        for uid in [3012011,12011]:
            after=parse_power_allowlist(BASE+f'user,{GMS},{uid}\n')
            verify_power_allowlist_delta(before,after,3012011,True)
            verify_power_allowlist_delta(before,before,3012011,False)

    def test_unrelated_store_iris_removal_or_wrong_uid_delta_rejected(self):
        before=parse_power_allowlist(BASE)
        for value in [BASE+'user,com.android.vending,12012\n', BASE+'user,to.iris.chat.debug,11829\n',
                      BASE+f'user,{GMS},12012\n', f'user,{GMS},12011\n', BASE]:
            with self.subTest(value=value),self.assertRaises(ValueError):
                verify_power_allowlist_delta(before,parse_power_allowlist(value),3012011,True)

    def test_unknown_duplicate_or_malformed_allowlist_is_not_empty_success(self):
        for value in ['', 'Unknown command', BASE+BASE, 'user,com.google.android.gms,not-uid\n']:
            with self.subTest(value=value),self.assertRaises(ValueError):parse_power_allowlist(value)

    def test_navigation_label_must_be_separate_from_actual_enabled_on_switch(self):
        good=page()
        for xml in [good.replace('[20,120][300,160]','[0,100][600,200]'),
                    good.replace('[400,120][500,160]','[200,120][500,160]'),
                    good.replace('checked="true"','checked="false"'),
                    good.replace('switchWidget','unrelatedSwitch'),
                    good.replace('android.widget.Switch','android.widget.CheckBox')]:
            with self.subTest(xml=xml),self.assertRaises(ValueError):battery_setting_control(xml,'Unrestricted')

    def test_observed_default_allow_and_explicit_allow_are_supported(self):
        for op in ['RUN_ANY_IN_BACKGROUND','RUN_IN_BACKGROUND']:
            self.assertTrue(effective_background_allowed('No operations.\nDefault mode: allow',op))
            self.assertTrue(effective_background_allowed(op+': allow',op))
            self.assertTrue(effective_background_allowed('Uid mode: '+op+': allow; time=+3s ago',op))
            for text in ['No operations.', 'Default mode: allow', 'No operations.\nDefault mode: ignore',
                         op+': ignore',op+': deny',op+': default',op+': allow\n'+op+': ignore','unknown']:
                with self.subTest(text=text),self.assertRaises(ValueError):effective_background_allowed(text,op)

    def test_only_exact_semantic_empty_appops_for_uninstalled_user_is_accepted(self):
        self.assertTrue(empty_uninstalled_appops(0,'No operations.\n','',0))
        self.assertTrue(empty_uninstalled_appops(255,'','Error: No UID for com.google.android.gms in user 0\n',0))
        for code,out,err in [(0,'Uid mode: COARSE_LOCATION: ignore\n',''),
                             (0,'RUN_ANY_IN_BACKGROUND: allow',''),(255,'','permission denied'),
                             (255,'','Error: No UID for com.google.android.gms in user 10\n'),
                             (0,'No operations.','warning'),(1,'No operations.','')]:
            with self.subTest(code=code,out=out,err=err),self.assertRaises(ValueError):empty_uninstalled_appops(code,out,err,0)

    def test_every_preexisting_user_must_have_no_gms(self):
        require_gms_absent({0:'',10:''})
        for snapshot in [{}, {0:'',10:'package:com.google.android.gms'}, {0:'unexpected failure'}]:
            with self.subTest(snapshot=snapshot),self.assertRaises(ValueError):require_gms_absent(snapshot)


if __name__=='__main__':unittest.main()
