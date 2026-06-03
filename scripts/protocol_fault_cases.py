#!/usr/bin/env python3
from __future__ import annotations

import time
from pathlib import Path

from protocol_fault_common import CaseResult, ValidationFailure


class ProtocolFaultCasesMixin:
    def run_revision_repair_flow(
        self,
        case_dir: Path,
        *,
        label: str,
        message_count: int = 1,
        offline_messages: bool = False,
        receiver_restart: bool = False,
        sender_restart: bool = False,
        linked_owner_assertion: bool = False,
    ) -> CaseResult:
        chat_id, group_id = self.base_group()
        stamp = case_stamp()
        baseline = f"{label}-baseline-{stamp}"
        new_name = f"{label} Revision {stamp}"
        messages = [f"{label}-message-{stamp}-{index}" for index in range(1, message_count + 1)]

        self.send_message(case_dir, "alice1", chat_id, baseline, suffix="baseline-send")
        self.wait_message(case_dir, "bob1", chat_id, baseline, suffix="baseline-wait-bob")

        self.begin_fault(case_dir)
        self.update_group_name(
            case_dir,
            group_id,
            new_name,
            wait_for_relay_drain=False,
            suffix="offline-rename",
        )
        if offline_messages:
            for message in messages:
                self.send_message(
                    case_dir,
                    "alice1",
                    chat_id,
                    message,
                    wait_for_delivery=False,
                    wait_for_relay_drain=False,
                    suffix=f"offline-send-{message}",
                )

        rows = self.pending_rows(
            case_dir,
            "alice1",
            peer_device_id="bob1",
            pairwise_only=True,
            suffix="bob-pending-before-drop",
        )
        drop_row = self.select_pending_row(rows, selector="newest", purpose="Bob metadata revision")
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-drop")
        self.wait_group_name(case_dir, "alice2", chat_id, new_name, suffix="alice2-renamed")

        if not offline_messages:
            for message in messages:
                self.send_message(case_dir, "alice1", chat_id, message, suffix=f"send-{message}")

        passive_ok = self.wait_message(
            case_dir,
            "bob1",
            chat_id,
            messages[-1],
            check=False,
            suffix="bob-passive-wait-last-message",
        )
        passive_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Bob missed the message but did not record pending sender-key repair state")

        if receiver_restart:
            self.restart_app(case_dir, "bob1", suffix="bob-restart-before-repair")
            restarted = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-restart")
            if self.pending_repair_count(restarted) < passive_pending:
                raise ValidationFailure("receiver restart lost pending sender-key repair state")

        if sender_restart:
            self.restart_app(case_dir, "alice1", suffix="alice-restart-before-repair")
            self.report_protocol_debug(case_dir, "alice1", "alice-debug-after-restart")

        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-force-request")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice1-alice-force-response")
        self.activate_connected(
            case_dir,
            "alice2",
            drain=linked_owner_assertion,
            suffix="alice2-alice-force-response",
        )
        self.activate_connected(case_dir, "bob1", drain=False, suffix="bob-force-response")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice1-alice-force-followup-response")
        self.activate_connected(
            case_dir,
            "alice2",
            drain=linked_owner_assertion,
            suffix="alice2-alice-force-followup-response",
        )
        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-force-followup-response")
        time.sleep(3)
        for message in messages:
            self.wait_message(case_dir, "bob1", chat_id, message, suffix=f"bob-final-wait-{message}")
        self.wait_group_name(case_dir, "bob1", chat_id, new_name, suffix="bob-final-name")
        if linked_owner_assertion:
            self.wait_group_name(case_dir, "alice2", chat_id, new_name, suffix="alice2-final-name")
            for message in messages:
                self.wait_message(
                    case_dir,
                    "alice2",
                    chat_id,
                    message,
                    direction="incoming",
                    suffix=f"alice2-final-wait-{message}",
                )

        final_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-final")
        final_pending = self.pending_repair_count(final_debug)
        for message in messages:
            count = self.visible_message_count("bob1", chat_id, message)
            if count != 1:
                raise ValidationFailure(f"expected Bob to have exactly one `{message}`, found {count}")

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "new_name": new_name,
                "messages": messages,
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def run_distribution_repair_flow(
        self,
        case_dir: Path,
        *,
        label: str,
        message_count: int = 1,
        receiver_restart: bool = False,
        sender_restart: bool = False,
    ) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"{label} Group {stamp}", ["bob1", "carol1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        baseline = f"{label}-baseline-{stamp}"
        messages = [f"{label}-after-rotation-{stamp}-{index}" for index in range(1, message_count + 1)]

        self.send_message(case_dir, "alice1", chat_id, baseline, suffix="baseline-send")
        self.wait_message(case_dir, "bob1", chat_id, baseline, suffix="baseline-wait-bob")
        self.wait_message(case_dir, "carol1", chat_id, baseline, suffix="baseline-wait-carol")

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            peer_device_id="bob1",
            pairwise_only=True,
            suffix="bob-pending-after-remove",
        )
        drop_row = self.select_sender_key_distribution_row(
            rows,
            purpose="Bob rotated sender-key distribution",
        )
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-distribution-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-remove")
        self.wait_member_count(case_dir, "bob1", chat_id, 2, suffix="bob-sees-removal")

        for message in messages:
            self.send_message(case_dir, "alice1", chat_id, message, suffix=f"send-after-rotation-{message}")
        passive_ok = self.wait_message(
            case_dir,
            "bob1",
            chat_id,
            messages[-1],
            check=False,
            suffix="bob-passive-wait-rotated-message",
        )
        passive_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Bob missed the rotated-key message but did not record pending sender-key repair state")

        if receiver_restart:
            self.restart_app(case_dir, "bob1", suffix="bob-restart-before-distribution-repair")
            restarted = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-restart")
            if self.pending_repair_count(restarted) < passive_pending:
                raise ValidationFailure("receiver restart lost pending sender-key distribution repair")

        if sender_restart:
            self.restart_app(case_dir, "alice1", suffix="alice-restart-before-distribution-repair")
            self.report_protocol_debug(case_dir, "alice1", "alice-debug-after-restart")

        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-force-request")
        self.activate_user_devices(case_dir, "alice", drain=True, suffix="alice-force-response")
        self.activate_connected(case_dir, "bob1", drain=True, suffix="bob-force-response")
        self.activate_user_devices(case_dir, "alice", drain=True, suffix="alice-force-followup-response")
        self.activate_connected(case_dir, "bob1", drain=False, suffix="bob-force-followup-response")
        for message in messages:
            self.wait_message(case_dir, "bob1", chat_id, message, suffix=f"bob-final-message-{message}")
            self.assert_message_absent(case_dir, "carol1", chat_id, message, suffix=f"carol-removed-absent-{message}")
        final_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-final")
        final_pending = self.pending_repair_count(final_debug)
        for message in messages:
            count = self.visible_message_count("bob1", chat_id, message)
            if count != 1:
                raise ValidationFailure(f"expected Bob to have exactly one `{message}`, found {count}")

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "messages": messages,
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def case_sender_key_revision_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="revision-repair")

    def case_sender_key_repair_after_receiver_restart(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="receiver-restart", receiver_restart=True)

    def case_sender_key_repair_after_sender_restart(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="sender-restart", sender_restart=True)

    def case_sender_key_duplicate_replay_idempotent(self, case_dir: Path) -> CaseResult:
        result = self.run_revision_repair_flow(case_dir, label="duplicate-replay")
        chat_id = result.details["group_chat_id"]
        message = result.details["messages"][0]
        self.wait_message(case_dir, "bob1", chat_id, message, suffix="bob-rewait-message")
        self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-rewait")
        count = self.visible_message_count("bob1", chat_id, message)
        if count != 1:
            raise ValidationFailure(f"duplicate replay should leave one message, found {count}")
        result.details["bob_message_count_after_rewait"] = count
        return result

    def case_group_metadata_drop_then_multiple_messages(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(case_dir, label="multi-message-revision", message_count=3)

    def case_relay_offline_outbox_then_repair(self, case_dir: Path) -> CaseResult:
        return self.run_revision_repair_flow(
            case_dir,
            label="offline-outbox",
            message_count=2,
            offline_messages=True,
        )

    def case_sender_key_distribution_repair(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(case_dir, label="distribution-repair")

    def case_sender_key_distribution_repair_after_receiver_restart(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-receiver-restart",
            receiver_restart=True,
        )

    def case_sender_key_distribution_repair_after_sender_restart(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-sender-restart",
            sender_restart=True,
        )

    def case_sender_key_distribution_duplicate_replay_idempotent(self, case_dir: Path) -> CaseResult:
        result = self.run_distribution_repair_flow(case_dir, label="distribution-duplicate")
        chat_id = result.details["group_chat_id"]
        message = result.details["messages"][0]
        self.wait_message(case_dir, "bob1", chat_id, message, suffix="bob-rewait-distribution-message")
        self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-distribution-rewait")
        count = self.visible_message_count("bob1", chat_id, message)
        if count != 1:
            raise ValidationFailure(f"duplicate distribution repair replay should leave one message, found {count}")
        result.details["bob_message_count_after_rewait"] = count
        return result

    def case_sender_key_distribution_multiple_messages(self, case_dir: Path) -> CaseResult:
        return self.run_distribution_repair_flow(
            case_dir,
            label="distribution-multi-message",
            message_count=3,
        )

    def case_sender_key_late_member_post_add_repair(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Late Add Repair {stamp}", ["bob1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        message = f"late-member-post-add-rotation-{stamp}"

        self.add_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=3,
            wait_for_relay_drain=True,
        )
        self.wait_member_count(case_dir, "carol1", chat_id, 3, suffix="carol-sees-initial-add")

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "bob1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            peer_device_id="carol1",
            pairwise_only=True,
            suffix="carol-pending-after-post-add-rotation",
        )
        drop_row = self.select_sender_key_distribution_row(
            rows,
            purpose="Carol post-add rotated sender-key distribution",
        )
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-post-add-rotation-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-post-add-rotation")
        self.wait_member_count(case_dir, "carol1", chat_id, 2, suffix="carol-sees-post-add-rotation")

        self.send_message(case_dir, "alice1", chat_id, message, suffix="send-post-add-rotation")
        passive_ok = self.wait_message(
            case_dir,
            "carol1",
            chat_id,
            message,
            check=False,
            suffix="carol-passive-wait-post-add",
        )
        passive_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-after-passive")
        passive_pending = self.pending_repair_count(passive_debug)
        if not passive_ok and passive_pending == 0:
            raise ValidationFailure("Carol missed the post-add message but did not record pending sender-key repair state")
        self.activate_connected(case_dir, "carol1", drain=True, suffix="carol-force-request")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-force-response")
        self.activate_connected(case_dir, "carol1", drain=False, suffix="carol-force-response")
        self.wait_message(case_dir, "carol1", chat_id, message, suffix="carol-final-message")
        final_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-final")
        final_pending = self.pending_repair_count(final_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=passive_pending > 0 or passive_ok,
            visible_result_ok=True,
            final_pending_repair_count=final_pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "message": message,
                "dropped_state": "post_add_rotation_distribution",
                "passive_success": passive_ok,
                "passive_pending_repair_count": passive_pending,
            },
        )

    def case_sender_key_removed_member_repair_denied(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Removed Repair Denied {stamp}", ["bob1", "carol1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        message = f"removed-member-future-{stamp}"

        self.begin_fault(case_dir)
        self.remove_group_member(
            case_dir,
            group_id,
            chat_id,
            "bob1",
            expected_member_count=2,
            wait_for_relay_drain=False,
        )
        rows = self.pending_rows(
            case_dir,
            "alice1",
            peer_device_id="bob1",
            pairwise_only=True,
            suffix="bob-removal-pending",
        )
        drop_row = self.select_pending_row(rows, selector="newest", purpose="Bob removal metadata")
        self.write_drop_file(drop_row["event_id"])
        self.start_relay(case_dir / "restart-relay-after-bob-removal-drop.log")
        self.activate_connected(case_dir, "alice1", drain=True, suffix="alice-after-remove-bob")

        self.send_message(case_dir, "alice1", chat_id, message, suffix="send-after-bob-remove")
        self.wait_message(case_dir, "carol1", chat_id, message, suffix="carol-receives-after-remove")
        self.assert_message_absent(case_dir, "bob1", chat_id, message, timeout_ms=30000, suffix="bob-removed-absent")
        bob_debug = self.report_protocol_debug(case_dir, "bob1", "bob-debug-after-denied")
        pending = self.pending_repair_count(bob_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=True,
            repair_observed=pending > 0,
            visible_result_ok=True,
            final_pending_repair_count=pending,
            dropped_event_id=drop_row["event_id"],
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "message": message,
                "bob_pending_repair_count_after_denial": pending,
            },
        )

    def case_sender_key_late_member_pre_add_denied(self, case_dir: Path) -> CaseResult:
        stamp = case_stamp()
        group = self.create_group(case_dir, f"Late Pre Add Denied {stamp}", ["bob1"])
        chat_id = group["chat_id"]
        group_id = group["group_id"]
        pre_add = f"late-member-pre-add-{stamp}"
        post_add = f"late-member-post-add-visible-{stamp}"

        self.send_message(case_dir, "alice1", chat_id, pre_add, suffix="send-pre-add")
        self.wait_message(case_dir, "bob1", chat_id, pre_add, suffix="bob-pre-add")
        self.add_group_member(
            case_dir,
            group_id,
            chat_id,
            "carol1",
            expected_member_count=3,
            wait_for_relay_drain=True,
        )
        self.wait_member_count(case_dir, "carol1", chat_id, 3, suffix="carol-sees-add")
        self.assert_message_absent(case_dir, "carol1", chat_id, pre_add, timeout_ms=30000, suffix="carol-pre-add-absent")
        self.send_message(case_dir, "alice1", chat_id, post_add, suffix="send-post-add-visible")
        self.wait_message(case_dir, "carol1", chat_id, post_add, suffix="carol-post-add-visible")
        carol_debug = self.report_protocol_debug(case_dir, "carol1", "carol-debug-final")
        pending = self.pending_repair_count(carol_debug)

        return CaseResult(
            case="",
            status="passed",
            fault_injected=False,
            repair_observed=pending > 0,
            visible_result_ok=True,
            final_pending_repair_count=pending,
            details={
                "group_chat_id": chat_id,
                "group_id": group_id,
                "pre_add_message": pre_add,
                "post_add_message": post_add,
            },
        )
