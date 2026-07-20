#!/usr/bin/env python3

import json
import tempfile
import unittest
from pathlib import Path

from stateful_soak_core import (
    Action,
    ActionGenerator,
    DeviceModel,
    GroupModel,
    Journal,
    WorldModel,
    parse_duration,
    read_replay_actions,
    safe_value,
)
from mobile_scenario_support import redact_sensitive_text


class DurationTests(unittest.TestCase):
    def test_supported_units(self) -> None:
        self.assertEqual(parse_duration("30"), 30)
        self.assertEqual(parse_duration("30s"), 30)
        self.assertEqual(parse_duration("2m"), 120)
        self.assertEqual(parse_duration("1.5h"), 5400)

    def test_invalid_duration_is_rejected(self) -> None:
        with self.assertRaises(Exception):
            parse_duration("forever")


class ModelTests(unittest.TestCase):
    def test_initial_model_has_three_users_and_one_deferred_device(self) -> None:
        model = WorldModel.initial(7)
        self.assertEqual(model.active_devices(), ["alice1", "bob1", "carol1"])
        self.assertFalse(model.devices["alice2"].provisioned)
        self.assertEqual(model.primary("alice"), "alice1")

    def test_digest_is_independent_of_set_insertion_order(self) -> None:
        first = WorldModel.initial(7)
        second = WorldModel.initial(7)
        first.coverage.update(("direct_send", "audit"))
        second.coverage.update(("audit", "direct_send"))
        self.assertEqual(first.digest(), second.digest())

    def test_group_serialization_is_stable(self) -> None:
        group = GroupModel(
            symbol="group-1",
            name="Test",
            creator="alice",
            members={"bob", "alice"},
            admins={"alice"},
            ever_members={"bob", "alice"},
        )
        self.assertEqual(group.as_dict()["members"], ["alice", "bob"])


class GeneratorTests(unittest.TestCase):
    def test_coverage_prefix_starts_with_link_direct_and_group(self) -> None:
        model = WorldModel.initial(123)
        generator = ActionGenerator(123, max_groups=2)

        first = generator.next(model, 1)
        self.assertEqual(first.kind, "link_device")
        model.coverage.add(first.kind)
        model.devices["alice2"] = DeviceModel("alice", True, True)

        second = generator.next(model, 2)
        self.assertEqual(second.kind, "direct_send")
        model.coverage.add(second.kind)

        third = generator.next(model, 3)
        self.assertEqual(third.kind, "create_group")

    def test_generator_is_deterministic_for_a_seed(self) -> None:
        first_model = WorldModel.initial(99)
        second_model = WorldModel.initial(99)
        first_model.coverage.update(ActionGenerator.COVERAGE_ORDER)
        second_model.coverage.update(ActionGenerator.COVERAGE_ORDER)
        first = ActionGenerator(99, 1).next(first_model, 1)
        second = ActionGenerator(99, 1).next(second_model, 1)
        self.assertEqual(first, second)

    def test_remove_requires_more_than_two_members(self) -> None:
        model = WorldModel.initial(4)
        generator = ActionGenerator(4, 1)
        model.groups["group-1"] = GroupModel(
            symbol="group-1",
            name="Group",
            creator="alice",
            members={"alice", "bob"},
            admins={"alice"},
            ever_members={"alice", "bob"},
        )
        self.assertNotIn("remove_group_member", generator.eligible(model))
        model.groups["group-1"].members.add("carol")
        model.groups["group-1"].ever_members.add("carol")
        self.assertIn("remove_group_member", generator.eligible(model))


class JournalTests(unittest.TestCase):
    def test_planned_action_survives_without_a_finished_record(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "actions.jsonl"
            journal = Journal(path)
            journal.append("run_started", run={"seed": 42, "profile": "ios-local"})
            expected = Action("direct_send", {"sender": "alice1", "recipient": "bob", "message": "marker"})
            journal.append("semantic_action_planned", sequence=1, action=expected.as_dict())
            journal.close()

            actions, metadata = read_replay_actions(path)
            self.assertEqual(actions, [expected])
            self.assertEqual(metadata["seed"], 42)

            records = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
            self.assertEqual([record["event"] for record in records], [1, 2])

    def test_sensitive_values_are_redacted_recursively(self) -> None:
        value = {
            "secret_key": "do-not-store",
            "nested": {"value": "nsec1example", "message": "safe"},
        }
        self.assertEqual(safe_value(value)["secret_key"], "<redacted>")
        self.assertEqual(safe_value(value)["nested"]["value"], "<redacted>")
        self.assertEqual(safe_value(value)["nested"]["message"], "safe")

    def test_device_approval_payloads_are_redacted(self) -> None:
        url = "nostr-identity://device-approval/opaque-secret"
        rendered = redact_sensitive_text(f"--arg device_input={url}\nHARNESS_STATUS: link_url={url}\n")
        self.assertNotIn("opaque-secret", rendered)
        self.assertIn("device_input=<redacted>", rendered)
        self.assertIn("link_url=<redacted>", rendered)
        self.assertEqual(safe_value({"invite_url": url})["invite_url"], "<redacted>")


if __name__ == "__main__":
    unittest.main()
