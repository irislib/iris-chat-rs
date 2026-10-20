import hashlib
import json
import unittest
import subprocess
import xml.etree.ElementTree as ET

from android_call_push_e2e import canceled_before_no_answer, ended_with_outcome, matched_call_callback, notification_action_node, window_help_supports_dismissal


class CallPushAcceptanceTest(unittest.TestCase):
    def test_timeout_cannot_pass_as_decline(self):
        status = {"call": {"id": "third", "phase": "ended"}}
        history = {"call_id": "third", "direction": "outgoing", "outcome": "canceled"}
        self.assertFalse(ended_with_outcome(status, history, "third", "declined"))
        history["outcome"] = "declined"
        self.assertTrue(ended_with_outcome(status, history, "third", "declined"))

    def test_other_call_or_active_history_cannot_pass_cancel(self):
        status = {"call": {"id": "fourth", "phase": "outgoing"}}
        history = {"call_id": "fourth", "direction": "outgoing", "outcome": "canceled"}
        self.assertFalse(ended_with_outcome(status, history, "fourth", "canceled"))
        status["call"]["phase"] = "ended"
        self.assertTrue(ended_with_outcome(status, history, "fourth", "canceled"))
        history["call_id"] = "third"
        self.assertFalse(ended_with_outcome(status, history, "fourth", "canceled"))

    def test_native_answer_ignores_duplicate_in_app_control(self):
        tree = ET.fromstring('<hierarchy><node package="to.iris.chat.debug" content-desc="Answer" enabled="true"/>'
                             '<node package="com.android.systemui" content-desc="Answer" enabled="true"/></hierarchy>')
        self.assertEqual(notification_action_node(tree, ("Answer", "Video")).get("package"), "com.android.systemui")

    def test_missing_or_disabled_system_action_cannot_select_app_button(self):
        tree = ET.fromstring('<hierarchy><node package="to.iris.chat.debug" content-desc="Answer" enabled="true"/>'
                             '<node package="com.android.systemui" content-desc="Answer" enabled="false"/></hierarchy>')
        self.assertIsNone(notification_action_node(tree, ("Answer", "Video")))

    def test_ambiguous_system_notification_actions_are_rejected(self):
        tree = ET.fromstring('<hierarchy><node package="com.android.systemui" content-desc="Answer" enabled="true"/>'
                             '<node package="com.android.systemui" content-desc="Video" enabled="true"/></hierarchy>')
        with self.assertRaisesRegex(AssertionError, "Ambiguous"):
            notification_action_node(tree, ("Answer", "Video"))

    def test_nonzero_readonly_help_with_advertised_command_is_supported(self):
        result = subprocess.CompletedProcess([], 255,
            "Window manager (window) commands:\n  dismiss-keyguard\n    Dismiss the keyguard.\n", "")
        self.assertTrue(window_help_supports_dismissal(result))

    def test_unknown_command_or_missing_help_advertisement_is_rejected(self):
        result = subprocess.CompletedProcess([], 255, "", "Unknown command: dismiss-keyguard")
        self.assertFalse(window_help_supports_dismissal(result))
        result = subprocess.CompletedProcess([], 0, "Window manager (window) commands:\n  size\n", "")
        self.assertFalse(window_help_supports_dismissal(result))

    def records(self):
        raw = json.dumps({"event": json.dumps({"id": "outer", "kind": 21111})})
        shared = {"outer_event_id": "outer", "raw_payload_sha256": hashlib.sha256(raw.encode()).hexdigest()}
        return [dict(shared, stage="fcm_received", raw_payload_json=raw),
                dict(shared, stage="resolved", call_id="third", raw_payload_json=raw,
                     preview_call={"call_id": "third", "phase": "incoming"})]

    def test_no_answer_cannot_pass_as_actual_caller_cancel(self):
        status = {"call": {"id": "fourth", "phase": "ended"}}
        history = {"call_id": "fourth", "direction": "outgoing", "outcome": "canceled",
                   "started_at_secs": 100, "ended_at_secs": 130}
        self.assertFalse(canceled_before_no_answer(status, history, "fourth", 129000))
        history["ended_at_secs"] = 111
        self.assertTrue(canceled_before_no_answer(status, history, "fourth", 110000))
        self.assertFalse(canceled_before_no_answer(status, history, "fourth", 112000))

    def test_original_callback_and_authenticated_preview_match(self):
        self.assertIsNotNone(matched_call_callback(self.records(), "third"))

    def test_other_call_or_missing_original_callback_cannot_pass(self):
        records = self.records()
        self.assertIsNone(matched_call_callback(records, "fourth"))
        self.assertIsNone(matched_call_callback(records[1:], "third"))
        records[1]["preview_call"]["call_id"] = "fourth"
        self.assertIsNone(matched_call_callback(records, "third"))

    def test_altered_payload_or_unrelated_callback_cannot_pass(self):
        records = self.records()
        records[1]["raw_payload_json"] += " "
        self.assertIsNone(matched_call_callback(records, "third"))
        records = self.records()
        records[0]["outer_event_id"] = "older-warm-call"
        self.assertIsNone(matched_call_callback(records, "third"))


if __name__ == "__main__":
    unittest.main()
