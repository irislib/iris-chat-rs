#!/usr/bin/env python3
"""Read one known notification subscription from a compatible LMDB library.

This private release diagnostic does not enumerate records, alter subscriptions,
or send notifications. Production credentials and identifiers belong on stdin.
LMDB's normal reader lock is retained so concurrent writers remain supported.
"""
from __future__ import annotations

import ctypes as C
import hashlib
import json
from pathlib import Path
import re
import sys

READ_ONLY = 0x20000
NOT_FOUND = -30798
MAX_RECORD_BYTES = 1024 * 1024


class Value(C.Structure):
    _fields_ = [('size', C.c_size_t), ('data', C.c_void_p)]


def library(path):
    lib = C.CDLL(str(path))
    signatures = {
        'mdb_env_create': ([C.POINTER(C.c_void_p)], C.c_int),
        'mdb_env_set_maxdbs': ([C.c_void_p, C.c_uint], C.c_int),
        'mdb_env_open': ([C.c_void_p, C.c_char_p, C.c_uint, C.c_uint], C.c_int),
        'mdb_txn_begin': ([C.c_void_p, C.c_void_p, C.c_uint, C.POINTER(C.c_void_p)], C.c_int),
        'mdb_dbi_open': ([C.c_void_p, C.c_char_p, C.c_uint, C.POINTER(C.c_uint)], C.c_int),
        'mdb_get': ([C.c_void_p, C.c_uint, C.POINTER(Value), C.POINTER(Value)], C.c_int),
        'mdb_txn_abort': ([C.c_void_p], None),
        'mdb_env_close': ([C.c_void_p], None),
        'mdb_version': ([C.POINTER(C.c_int)] * 3, C.c_char_p),
    }
    for name, (args, result) in signatures.items():
        function = getattr(lib, name)
        function.argtypes, function.restype = args, result
    major, minor, patch = C.c_int(), C.c_int(), C.c_int()
    lib.mdb_version(C.byref(major), C.byref(minor), C.byref(patch))
    if (major.value, minor.value, patch.value) != (0, 9, 70):
        raise ValueError('The qualified LMDB version is required')
    return lib


def read_one(lib, directory, subscription_id, trace):
    if not isinstance(subscription_id, str) or not re.fullmatch(r'[A-Za-z0-9-]{1,128}', subscription_id):
        raise ValueError('Invalid exact subscription ID')
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir() or not (directory / 'data.mdb').is_file():
        raise ValueError('Existing database directory is required')
    env, txn, dbi = C.c_void_p(), C.c_void_p(), C.c_uint()

    def call(name, *args, flags=None):
        trace.append({'name': name, 'flags': flags})
        result = getattr(lib, name)(*args)
        if result not in (None, 0):
            raise RuntimeError(f'{name} failed with code {result}')

    try:
        call('mdb_env_create', C.byref(env))
        call('mdb_env_set_maxdbs', env, 32)
        call('mdb_env_open', env, str(directory).encode(), READ_ONLY, 0, flags=READ_ONLY)
        call('mdb_txn_begin', env, None, READ_ONLY, C.byref(txn), flags=READ_ONLY)
        call('mdb_dbi_open', txn, b'subscriptions', 0, C.byref(dbi), flags=0)
        encoded = subscription_id.encode('ascii')
        key_buffer = C.create_string_buffer(encoded)
        key = Value(len(encoded), C.cast(key_buffer, C.c_void_p))
        value = Value()
        trace.append({'name': 'mdb_get', 'flags': None})
        result = lib.mdb_get(txn, dbi, C.byref(key), C.byref(value))
        if result == NOT_FOUND:
            return None
        if result != 0:
            raise RuntimeError(f'mdb_get failed with code {result}')
        if not 0 < value.size <= MAX_RECORD_BYTES:
            raise ValueError('Subscription record outside bounded size')
        raw = C.string_at(value.data, value.size)
        record = json.loads(raw)
        if not isinstance(record, dict):
            raise ValueError('Expected the current JSON subscription format')
        return raw
    finally:
        if txn.value:
            call('mdb_txn_abort', txn)
        if env.value:
            call('mdb_env_close', env)


def main():
    data = sys.stdin.buffer.read(16385)
    if len(data) > 16384:
        raise ValueError('Oversized private input')
    config = json.loads(data)
    if set(config) != {'library', 'library_sha256', 'database', 'subscription_id'}:
        raise ValueError('Unexpected private input')
    path = Path(config['library'])
    if path.is_symlink() or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != config['library_sha256']:
        raise ValueError('Library differs from qualified bytes')
    trace = []
    raw = read_one(library(path), config['database'], config['subscription_id'], trace)
    result = {'schema': 'iris-exact-subscription-reader-v1', 'read_only': True,
              'point_reads': 1, 'record_count': int(raw is not None), 'trace': trace,
              'record': None if raw is None else json.loads(raw),
              'raw_record_sha256': None if raw is None else hashlib.sha256(raw).hexdigest()}
    print(json.dumps(result, separators=(',', ':')))
    return 0 if raw is not None else 4


if __name__ == '__main__':
    raise SystemExit(main())
