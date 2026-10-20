import hashlib
import json
import unittest

from android_call_push_e2e import canceled_before_no_answer, ended_with_outcome, matched_call_callback


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
