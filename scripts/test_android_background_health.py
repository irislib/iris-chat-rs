#!/usr/bin/env python3
import unittest

from android_background_health import FLAGS, classify_health


class BackgroundHealthTest(unittest.TestCase):
    def healthy(self):
        return dict(schema_version=1, valid=True, pid=42, elapsed_realtime_ms=1234,
                    expected_relay_matches=True, configured_relay_count=1, connected_relay_count=1,
                    pending_relay_publish_count=0, retry_backoff_attempt=0, phase="connected",
                    **{key: False for key in FLAGS})

    def test_reachable_listener_is_not_enough(self):
        snapshot = self.healthy()
        self.assertEqual("connected-idle", classify_health(snapshot, 42, True))
        self.assertEqual("unavailable", classify_health(snapshot, 42, False))
        snapshot["connected_relay_count"] = 0
        snapshot["phase"] = "backoff"
        self.assertEqual("reconnecting", classify_health(snapshot, 42, True))
        snapshot["expected_relay_matches"] = False
        self.assertEqual("unavailable", classify_health(snapshot, 42, True))

    def test_queue_and_inflight_work_are_not_idle(self):
        for key, value, classification in [
            ("pending_relay_publish_count", 2, "draining"),
            ("publish_drain_in_flight", True, "draining"),
            ("publish_drain_dirty", True, "draining"),
            ("phase", "publishing", "draining"),
            ("retry_scheduled", True, "reconnecting"),
            ("connect_dirty", True, "reconnecting"),
        ]:
            with self.subTest(key=key):
                snapshot = self.healthy()
                snapshot[key] = value
                self.assertEqual(classification, classify_health(snapshot, 42, True))

    def test_missing_invalid_timeout_or_wrong_process_never_passes(self):
        for key in self.healthy():
            with self.subTest(missing=key):
                snapshot = self.healthy()
                del snapshot[key]
                with self.assertRaises(AssertionError):
                    classify_health(snapshot, 42, True)
        for key, value in [("valid", False), ("pid", 43), ("connected_relay_count", True),
                           ("pending_relay_publish_count", -1), ("retry_scheduled", "false")]:
            with self.subTest(invalid=key):
                snapshot = self.healthy()
                snapshot[key] = value
                with self.assertRaises(AssertionError):
                    classify_health(snapshot, 42, True)


if __name__ == "__main__":
    unittest.main()
