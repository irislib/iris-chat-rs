#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class ValidationFailure(Exception):
    pass


@dataclass
class CaseResult:
    case: str
    status: str
    fault_injected: bool = False
    repair_observed: bool = False
    visible_result_ok: bool = False
    final_pending_repair_count: int = 0
    artifact_dir: str = ""
    dropped_event_id: str = ""
    details: dict[str, Any] = field(default_factory=dict)
    error: str = ""

    def to_json(self) -> dict[str, Any]:
        result = {
            "case": self.case,
            "status": self.status,
            "fault_injected": self.fault_injected,
            "repair_observed": self.repair_observed,
            "visible_result_ok": self.visible_result_ok,
            "final_pending_repair_count": self.final_pending_repair_count,
            "artifact_dir": self.artifact_dir,
        }
        if self.dropped_event_id:
            result["dropped_event_id"] = self.dropped_event_id
        if self.details:
            result["details"] = self.details
        if self.error:
            result["error"] = self.error
        return result
