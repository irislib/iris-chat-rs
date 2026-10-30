"""Synthetic database checks; never opens the production notification database."""
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

import ios_notification_subscription_reader as reader


class InputTests(unittest.TestCase):
    def test_invalid_identifier_rejected_before_library_call(self):
        for value in ('', '../id', 'id\x00other', 'x' * 129, None):
            with self.assertRaises(ValueError):
                reader.read_one(None, '/unused', value, [])

    def test_missing_database_is_not_created(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'absent'
            with self.assertRaises(ValueError):
                reader.read_one(None, path, 'synthetic-id', [])
            self.assertFalse(path.exists())


@unittest.skipUnless(os.environ.get('IRIS_LMDB_TEST_LIBRARY'), 'A qualified native LMDB fixture library is required')
class NativeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)
        self.lib = reader.library(os.environ['IRIS_LMDB_TEST_LIBRARY'])
        self.lib.mdb_env_set_mapsize.argtypes = [C.c_void_p, C.c_size_t]
        self.lib.mdb_env_set_mapsize.restype = C.c_int
        self.lib.mdb_put.argtypes = [C.c_void_p, C.c_uint, C.POINTER(reader.Value), C.POINTER(reader.Value), C.c_uint]
        self.lib.mdb_put.restype = C.c_int
        self.lib.mdb_txn_commit.argtypes = [C.c_void_p]
        self.lib.mdb_txn_commit.restype = C.c_int
        env, txn, dbi = C.c_void_p(), C.c_void_p(), C.c_uint()
        self.assertEqual(self.lib.mdb_env_create(C.byref(env)), 0)
        try:
            self.assertEqual(self.lib.mdb_env_set_mapsize(env, 2 * 1024 * 1024), 0)
            self.assertEqual(self.lib.mdb_env_set_maxdbs(env, 32), 0)
            self.assertEqual(self.lib.mdb_env_open(env, str(self.path).encode(), 0, 0o600), 0)
            self.assertEqual(self.lib.mdb_txn_begin(env, None, 0, C.byref(txn)), 0)
            self.assertEqual(self.lib.mdb_dbi_open(txn, b'subscriptions', 0x40000, C.byref(dbi)), 0)
            for name, value in [('owned-id', {'apns_tokens': ['synthetic-owned-token']}),
                                ('unrelated-id', {'private': 'must-not-be-returned'}),
                                ('not-json', 'legacy-flatbuffer'), ('not-object', ['wrong'])]:
                key_bytes = name.encode()
                data = json.dumps(value).encode() if name != 'not-json' else b'not json'
                key_buffer, value_buffer = C.create_string_buffer(key_bytes), C.create_string_buffer(data)
                key = reader.Value(len(key_bytes), C.cast(key_buffer, C.c_void_p))
                val = reader.Value(len(data), C.cast(value_buffer, C.c_void_p))
                self.assertEqual(self.lib.mdb_put(txn, dbi, C.byref(key), C.byref(val), 0), 0)
            self.assertEqual(self.lib.mdb_txn_commit(txn), 0)
            txn = C.c_void_p()
        finally:
            if txn.value:
                self.lib.mdb_txn_abort(txn)
            self.lib.mdb_env_close(env)
        self.initial_hash = hashlib.sha256((self.path / 'data.mdb').read_bytes()).hexdigest()

    def check_trace(self, trace):
        self.assertEqual([x['name'] for x in trace], ['mdb_env_create', 'mdb_env_set_maxdbs',
            'mdb_env_open', 'mdb_txn_begin', 'mdb_dbi_open', 'mdb_get', 'mdb_txn_abort', 'mdb_env_close'])
        self.assertEqual(trace[2]['flags'], reader.READ_ONLY)
        self.assertEqual(trace[3]['flags'], reader.READ_ONLY)
        self.assertEqual(trace[4]['flags'], 0)
        self.assertEqual(hashlib.sha256((self.path / 'data.mdb').read_bytes()).hexdigest(), self.initial_hash)

    def test_exact_owned_key_only_with_readonly_flags_and_unchanged_data(self):
        trace = []
        raw = reader.read_one(self.lib, self.path, 'owned-id', trace)
        self.assertEqual(json.loads(raw), {'apns_tokens': ['synthetic-owned-token']})
        self.assertNotIn(b'must-not-be-returned', raw)
        self.check_trace(trace)

    def test_missing_key_returns_none_and_closes_read_transaction(self):
        trace = []
        self.assertIsNone(reader.read_one(self.lib, self.path, 'missing-id', trace))
        self.check_trace(trace)

    def test_bad_json_closes_and_preserves_database(self):
        trace = []
        with self.assertRaises(ValueError):
            reader.read_one(self.lib, self.path, 'not-json', trace)
        self.check_trace(trace)

    def test_non_object_record_rejected(self):
        trace = []
        with self.assertRaises(ValueError):
            reader.read_one(self.lib, self.path, 'not-object', trace)
        self.check_trace(trace)


if __name__ == '__main__':
    unittest.main()
