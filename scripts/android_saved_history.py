"""Private fingerprints prove test messages did not replace pre-existing history."""
import hashlib
import json


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode()).hexdigest()


def history_snapshot(connection):
    # Delivery/unread/reaction bookkeeping may change normally. Preserve message
    # identity, authored content/attachments, drafts, and all existing groups.
    threads = {fingerprint(chat): fingerprint(draft)
               for chat, draft in connection.execute("SELECT chat_id,draft FROM threads")}
    messages = {}
    for row in connection.execute("SELECT chat_id,id,kind,author,body,is_outgoing,created_at_secs,attachments_json FROM messages"):
        key = fingerprint(row[:2])
        assert key not in messages, "Duplicate saved message identity"
        messages[key] = dict(chat=fingerprint(row[0]), content=fingerprint(row))
    groups = {fingerprint(row[0]): fingerprint(row) for row in connection.execute(
        "SELECT group_id,name,picture,created_at_ms,group_json FROM groups")}
    return dict(threads=threads, messages=messages, groups=groups)


def require_preserved_history(before, after, allowed_new_chat=None):
    """Only the labelled fixture may add rows; equal totals cannot conceal loss."""
    additions = {}
    allowed = None if allowed_new_chat is None else fingerprint(allowed_new_chat)
    for table in ("threads", "messages", "groups"):
        old, new = before[table], after[table]
        assert all(key in new and new[key] == value for key, value in old.items()), "Existing history changed: " + table
        added = new.keys() - old.keys()
        if table == "groups" or allowed is None:
            assert not added, "Unexpected test history additions: " + table
        elif table == "threads":
            assert added <= {allowed}, "Unexpected new test contact"
        else:
            assert all(new[key]["chat"] == allowed for key in added), "Message added outside the test fixture"
        additions[table] = len(added)
    return additions
