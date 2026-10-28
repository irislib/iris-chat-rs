import hashlib
import http.client
import importlib.util
import io
import json
import os
from pathlib import Path
import ssl
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('timing', ROOT / 'scripts/public_source_timing.py')
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
BODIES = [b'public ciphertext one', b'public ciphertext two']
SAMPLES = [{'sha256': hashlib.sha256(b).hexdigest(), 'bytes': len(b)} for b in BODIES]


def environment():
    return {f'SAMPLE_{i}_{name}': str(sample[key])
            for i, sample in enumerate(SAMPLES, 1)
            for name, key in [('SHA256', 'sha256'), ('BYTES', 'bytes')]}


def wire(body=b'', status=200, headers=b''):
    return (f'HTTP/1.1 {status} Result\r\nContent-Length: {len(body)}\r\n'.encode()
            + headers + b'\r\n' + body)


class Socket:
    def __init__(self, data):
        self.data = data
        self.buffering = []

    def makefile(self, *_, buffering=None):
        self.buffering.append(buffering)
        return io.BytesIO(self.data)


class Connection:
    def __init__(self, host, budget, reply=None):
        self.host, self.budget, self.reply = host, budget, reply
        self.sock = None
        self.requests = []
        self.connect_seconds = self.tls_seconds = 0.0

    def request(self, method, target, headers):
        self.requests.append((method, target, headers))
        self.sock = object()

    def getresponse(self):
        if isinstance(self.reply, Exception):
            raise self.reply
        body = next(b for b, s in zip(BODIES, SAMPLES)
                    if self.requests[-1][1] == '/' + s['sha256'])
        response = M.TimedResponse(Socket(self.reply or wire(body)), budget=self.budget)
        response.begin()
        return response

    def close(self):
        self.sock = None


class PublicSourceTimingTests(unittest.TestCase):
    def test_strict_input_rejects_urls_uppercase_whitespace_zero_and_excess_sizes(self):
        for key, value in [('SAMPLE_1_SHA256', 'https://127.0.0.1/'),
                           ('SAMPLE_1_SHA256', 'A' * 64), ('SAMPLE_1_SHA256', 'a' * 64 + '\n'),
                           ('SAMPLE_2_BYTES', '0'), ('SAMPLE_2_BYTES', '01'),
                           ('SAMPLE_2_BYTES', '+1'), ('SAMPLE_2_BYTES', '1.0'),
                           ('SAMPLE_2_BYTES', str(M.MAX_SAMPLE_BYTES + 1)),
                           ('SAMPLE_2_BYTES', '1000000000000000000000')]:
            with self.subTest(key=key, value=value):
                env = environment(); env[key] = value
                with self.assertRaises(ValueError):
                    M.samples_from_env(env)
        self.assertEqual(M.samples_from_env(environment()), SAMPLES)
        env = environment(); env['SAMPLE_2_SHA256'] = env['SAMPLE_1_SHA256']
        with self.assertRaises(ValueError):
            M.samples_from_env(env)

    def test_largest_inputs_fit_two_rounds_three_hosts_with_header_allowance(self):
        total = 12 * (M.MAX_SAMPLE_BYTES + M.MAX_HEADER_BYTES)
        self.assertLessEqual(total, M.MAX_RESPONSE_BYTES)
        self.assertLess(12 * M.REQUEST_TIMEOUT, 5 * 60)

    def test_fixed_endpoints_exact_paths_and_connection_reuse(self):
        connections = []
        def factory(host, budget):
            connection = Connection(host, budget); connections.append(connection)
            return connection
        rows, size = M.probe(SAMPLES, factory)
        self.assertEqual([c.host for c in connections], list(M.HOSTS))
        self.assertEqual(len(rows), 12)
        self.assertTrue(all(r['expected_hash_match'] for r in rows))
        self.assertEqual(size, 6 * sum(len(wire(b)) for b in BODIES))
        self.assertEqual([r['connection_reused'] for r in rows], [False, True, True, True] * 3)
        for c in connections:
            self.assertEqual([r[1] for r in c.requests], ['/' + s['sha256'] for s in SAMPLES] * 2)
            self.assertTrue(all(r[0] == 'GET' and r[2] == {'Accept-Encoding': 'identity'} for r in c.requests))
        with self.assertRaises(ValueError):
            M.TimedConnection('127.0.0.1', {'bytes': 0})

    def test_tls_requires_valid_certificate_and_hostname(self):
        connection = M.TimedConnection(M.HOSTS[0], {'bytes': 0})
        self.assertEqual(connection._context.verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(connection._context.check_hostname)

    def test_redirect_not_followed_or_retried_and_headers_sanitized(self):
        private = b'Location: https://private.example/secret\r\nSet-Cookie: account=secret\r\n'
        response = wire(status=302, headers=private + b'X-Cache: HIT from 10.1.2.3\r\n'
                        b'Age: 20\r\nCache-Control: public, max-age=60, account=secret\r\n')
        connection = Connection(M.HOSTS[0], {'bytes': 0}, response)
        row = M.request(connection, M.HOSTS[0], SAMPLES[0], 1, 1, connection.budget)
        self.assertEqual(len(connection.requests), 1)
        self.assertEqual(row['http_code'], 302)
        self.assertEqual(row['error'], 'http_status')
        self.assertEqual(row['body_bytes'], 0)
        self.assertEqual(row['cache_headers'], {'age': '20', 'cache-control': 'public, max-age=60'})
        self.assertNotIn('secret', json.dumps(row)); self.assertNotIn('10.1.2.3', json.dumps(row))

    def test_wrong_size_or_hash_is_not_a_pass(self):
        for body in [b'x' * len(BODIES[0]), b'x', BODIES[0] + b'excess']:
            connection = Connection(M.HOSTS[0], {'bytes': 0}, wire(body))
            row = M.request(connection, M.HOSTS[0], SAMPLES[0], 1, 1, connection.budget)
            self.assertFalse(row['expected_hash_match'])
            self.assertEqual(row['error'], 'size_or_hash')
            self.assertLessEqual(row['body_bytes'], SAMPLES[0]['bytes'] + 1)
            self.assertIsNone(connection.sock)

    def test_global_budget_covers_headers_body_and_chunk_framing(self):
        chunked = b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n15\r\n' + BODIES[0] + b'\r\n0\r\n\r\n'
        for reply in [wire(BODIES[0]), chunked]:
            # Stop inside the status line, body, and final chunk framing.
            for cap in [10, len(reply) - 7, len(reply) - 1]:
                with self.subTest(reply=reply, cap=cap), patch.object(M, 'MAX_RESPONSE_BYTES', cap + 60):
                    budget = {'bytes': 60}
                    connection = Connection(M.HOSTS[0], budget, reply)
                    row = M.request(connection, M.HOSTS[0], SAMPLES[0], 1, 1, budget)
                    self.assertEqual(row['error'], 'response_limit')
                    self.assertEqual(budget['bytes'], cap + 60)
                    self.assertFalse(row['expected_hash_match'])
            budget = {'bytes': 0}
            row = M.request(Connection(M.HOSTS[0], budget, reply), M.HOSTS[0], SAMPLES[0], 1, 1, budget)
            self.assertTrue(row['expected_hash_match'])
            self.assertEqual(budget['bytes'], len(reply))
        budget = {'bytes': 0}
        reply = wire(BODIES[0], headers=b'X-Large: ' + b'x' * 10000 + b'\r\n')
        row = M.request(Connection(M.HOSTS[0], budget, reply), M.HOSTS[0], SAMPLES[0], 1, 1, budget)
        self.assertEqual(row['error'], 'response_limit')
        self.assertEqual(budget['bytes'], M.MAX_HEADER_BYTES)

    def test_ttfb_is_recorded_on_first_byte_not_end_of_status_line(self):
        socket = Socket(wire())
        response = M.TimedResponse(socket, budget={'bytes': 0})
        self.assertEqual(socket.buffering[-1], 0)
        with patch.object(M.time, 'monotonic', return_value=12.5):
            self.assertEqual(response.fp.readline(1), b'H')
        self.assertEqual(response.first_byte_at, 12.5)
        response.close()

    def test_exhausted_shared_budget_stops_later_requests(self):
        connections = []
        def factory(host, budget):
            connection = Connection(host, budget); connections.append(connection)
            return connection
        cap = len(wire(BODIES[0])) + 10
        with patch.object(M, 'MAX_RESPONSE_BYTES', cap):
            rows, size = M.probe(SAMPLES, factory)
        self.assertEqual(size, cap)
        self.assertEqual(len(rows), 2)
        self.assertEqual(len(connections), 1)
        self.assertEqual(len(connections[0].requests), 2)
        self.assertTrue(rows[0]['expected_hash_match'])
        self.assertEqual(rows[1]['error'], 'response_limit')

    def test_absolute_request_timeout_and_error_strings_do_not_escape(self):
        connection = Connection(M.HOSTS[0], {'bytes': 0})
        with patch.object(connection, 'getresponse', side_effect=lambda: time.sleep(.1)), \
             patch.object(M, 'REQUEST_TIMEOUT', .01):
            row = M.request(connection, M.HOSTS[0], SAMPLES[0], 1, 1, connection.budget)
        self.assertEqual(row['error'], 'timeout')
        connection = Connection(M.HOSTS[0], {'bytes': 0}, OSError('/private/path account@example 10.1.2.3'))
        row = M.request(connection, M.HOSTS[0], SAMPLES[0], 1, 1, connection.budget)
        self.assertEqual(row['error'], 'transport')
        self.assertNotIn('account', json.dumps(row)); self.assertNotIn('/private', json.dumps(row))

    def test_invalid_input_writes_small_failure_without_network_or_raw_input(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(M, 'OUTPUT', Path(temporary) / 'result.json'), \
             patch.dict(os.environ, {'SAMPLE_1_SHA256': '/private/path'}, clear=True), patch.object(M, 'probe') as probe:
            self.assertEqual(M.main(), 1)
            probe.assert_not_called()
            text = M.OUTPUT.read_text()
            self.assertNotIn('/private', text)
            self.assertEqual(json.loads(text)['error'], 'invalid_inputs')
            self.assertLess(len(text), 1024)

    def test_complete_output_is_small_and_failed_requests_remain_visible(self):
        real_probe = M.probe
        for reply, exit_code in [(None, 0), (wire(status=503), 1)]:
            def fake_probe(samples, rows):
                return real_probe(samples, lambda host, budget: Connection(host, budget, reply), rows)
            with self.subTest(exit_code=exit_code), tempfile.TemporaryDirectory() as temporary, \
                 patch.object(M, 'OUTPUT', Path(temporary) / 'result.json'), \
                 patch.dict(os.environ, environment(), clear=True), patch.object(M, 'probe', side_effect=fake_probe):
                self.assertEqual(M.main(), exit_code)
                text = M.OUTPUT.read_text()
                report = json.loads(text)
                self.assertEqual(len(report['results']), 12)
                self.assertEqual(report['passed'], exit_code == 0)
                self.assertEqual(report['response_bytes'], sum(row['response_bytes'] for row in report['results']))
                self.assertLess(len(text), 16000)
                self.assertNotIn(BODIES[0].decode(), text)
                self.assertNotIn(temporary, text)
                if exit_code:
                    self.assertTrue(all(row['error'] == 'http_status' for row in report['results']))

    def test_unexpected_error_preserves_prior_results_without_exception_text(self):
        def interrupted_probe(samples, rows):
            rows.append({'response_bytes': 12, 'expected_hash_match': False})
            raise RuntimeError('/private/path account@example')
        with tempfile.TemporaryDirectory() as temporary, patch.object(M, 'OUTPUT', Path(temporary) / 'result.json'), \
             patch.dict(os.environ, environment(), clear=True), patch.object(M, 'probe', side_effect=interrupted_probe):
            self.assertEqual(M.main(), 1)
            text = M.OUTPUT.read_text()
            report = json.loads(text)
            self.assertEqual(report['error'], 'internal_error')
            self.assertEqual(report['response_bytes'], 12)
            self.assertEqual(len(report['results']), 1)
            self.assertNotIn('/private/path', text)
            self.assertNotIn('account@example', text)

    def test_workflow_is_manual_read_only_and_retains_only_json_on_failure(self):
        text = (ROOT / '.github/workflows/public-source-timing.yml').read_text()
        self.assertIn('workflow_dispatch:', text)
        self.assertNotIn('pull_request:', text); self.assertNotIn('\n  push:', text)
        self.assertIn('runs-on: ubuntu-24.04', text)
        self.assertIn('persist-credentials: false', text)
        self.assertIn('contents: read', text)
        self.assertNotIn('secrets.', text)
        self.assertIn('if: always()', text)
        self.assertIn('path: public-source-timing.json', text)
        self.assertIn('actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1', text)
        self.assertIn('actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9', text)


if __name__ == '__main__':
    unittest.main()
