import json
import sqlite3
import unittest

from android_saved_history import history_snapshot, require_preserved_history


class SavedHistoryTest(unittest.TestCase):
    def setUp(self):
        self.connection = sqlite3.connect(":memory:")
        self.addCleanup(self.connection.close)
        self.connection.executescript("""
            CREATE TABLE threads(chat_id TEXT PRIMARY KEY,draft TEXT,unread_count INTEGER);
            CREATE TABLE messages(chat_id TEXT,id TEXT,kind TEXT,author TEXT,body TEXT,is_outgoing INTEGER,
                created_at_secs INTEGER,attachments_json TEXT,delivery TEXT);
            CREATE TABLE groups(group_id TEXT,name TEXT,picture TEXT,created_at_ms INTEGER,group_json TEXT);
            INSERT INTO threads VALUES('old-chat','private draft',0);
            INSERT INTO messages VALUES('old-chat','message-1','user','author','private message',0,1,'[]','received');
        """)

    def snapshot(self): return history_snapshot(self.connection)

    def test_preserves_old_content_and_only_allows_labelled_fixture_additions(self):
        before = self.snapshot()
        self.assertNotIn("private", json.dumps(before))
        self.connection.executescript("""
            UPDATE messages SET delivery='seen'; UPDATE threads SET unread_count=1;
            INSERT INTO threads VALUES('fixture','',0);
            INSERT INTO messages VALUES('fixture','test-1','user','fixture','test message',0,2,'[]','received');
        """)
        result = require_preserved_history(before, self.snapshot(), "fixture")
        self.assertEqual(dict(threads=1, messages=1, groups=0), result)
        with self.assertRaises(AssertionError): require_preserved_history(before, self.snapshot())

    def test_deletion_cannot_be_hidden_by_equal_count_new_messages(self):
        before = self.snapshot()
        self.connection.execute("UPDATE messages SET id='replacement'")
        with self.assertRaises(AssertionError): require_preserved_history(before, self.snapshot(), "old-chat")

    def test_existing_content_drafts_and_groups_cannot_change(self):
        for statement in ("UPDATE messages SET body='changed'", "UPDATE threads SET draft='changed'",
                          "UPDATE messages SET attachments_json='[1]'", "DELETE FROM threads"):
            before = self.snapshot()
            self.connection.execute("SAVEPOINT mutation")
            self.connection.execute(statement)
            with self.assertRaises(AssertionError): require_preserved_history(before, self.snapshot(), "fixture")
            self.connection.execute("ROLLBACK TO mutation")
            self.connection.execute("RELEASE mutation")

    def test_new_unrelated_chat_or_group_is_not_part_of_test_workload(self):
        before = self.snapshot()
        self.connection.execute("INSERT INTO threads VALUES('other','',0)")
        with self.assertRaises(AssertionError): require_preserved_history(before, self.snapshot(), "fixture")
        self.connection.execute("DELETE FROM threads WHERE chat_id='other'")
        self.connection.execute("INSERT INTO groups VALUES('group','name',NULL,1,'{}')")
        with self.assertRaises(AssertionError): require_preserved_history(before, self.snapshot(), "fixture")


if __name__ == "__main__": unittest.main()
