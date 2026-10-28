#!/usr/bin/env python3
"""Bounded HTTP/1.1 timing of two public ciphertext leaves; never retain bodies.

Only the three fixed public endpoints are contacted. Connect time includes DNS
and TCP; TLS is separate; TTFB is measured at the first HTTP response byte.
The shared byte limit includes status lines, headers, bodies and chunk framing
read by the HTTP parser, excluding TLS/network framing. This is a transport
diagnostic, not source verification or a benchmark of the Haps client.
"""
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import signal
import ssl
import time
from contextlib import contextmanager

HOSTS = ('blossom.primal.net', 'cdn.iris.to', 'upload.iris.to')
REQUEST_PATH_SUFFIX = '.bin'
ROUNDS = 2
MAX_SAMPLE_BYTES = 80 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
MAX_HEADER_BYTES = 4096
REQUEST_TIMEOUT = 15
OUTPUT = Path('public-source-timing.json')


class LimitExceeded(Exception):
    pass


def samples_from_env(environment):
    samples = []
    for number in (1, 2):
        digest = environment.get(f'SAMPLE_{number}_SHA256', '')
        size = environment.get(f'SAMPLE_{number}_BYTES', '')
        if not re.fullmatch(r'[0-9a-f]{64}', digest):
            raise ValueError('invalid sample hash')
        if not re.fullmatch(r'[1-9][0-9]{0,5}', size) or int(size) > MAX_SAMPLE_BYTES:
            raise ValueError('invalid sample size')
        samples.append({'sha256': digest, 'bytes': int(size)})
    if samples[0]['sha256'] == samples[1]['sha256']:
        raise ValueError('two distinct samples required')
    return samples


@contextmanager
def deadline():
    def expired(*_):
        raise TimeoutError()
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, REQUEST_TIMEOUT)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


class MeteredReader:
    """Meter parser reads without retaining bytes or exporting arbitrary headers."""
    def __init__(self, reader, budget, response):
        self.reader, self.budget, self.response = reader, budget, response
        self.headers = True
        self.header_bytes = 0

    def take(self, method, size):
        if size == 0:
            return b''
        remaining = MAX_RESPONSE_BYTES - self.budget['bytes']
        if self.headers:
            remaining = min(remaining, MAX_HEADER_BYTES - self.header_bytes)
        if remaining <= 0:
            raise LimitExceeded()
        value = method(min(remaining, size) if size >= 0 else remaining)
        self.budget['bytes'] += len(value)
        if self.headers:
            self.header_bytes += len(value)
        if value and self.response.first_byte_at is None:
            self.response.first_byte_at = time.monotonic()
        return value

    def readline(self, size=-1):
        # Read one byte first so TTFB does not include receipt of the whole line.
        if self.response.first_byte_at is None:
            first = self.take(self.reader.read, 1)
            if not first or first == b'\n' or size == 1:
                return first
            value = first + self.take(self.reader.readline, size - 1 if size > 0 else -1)
        else:
            value = self.take(self.reader.readline, size)
        if value and not value.endswith(b'\n') and (
                self.budget['bytes'] == MAX_RESPONSE_BYTES or
                (self.headers and self.header_bytes == MAX_HEADER_BYTES)):
            raise LimitExceeded()
        return value

    def read(self, size=-1):
        return self.take(self.reader.read, size)

    def read1(self, size=-1):
        return self.take(self.reader.read, size)

    def close(self):
        self.reader.close()

    def flush(self):
        self.reader.flush()


class TimedResponse(http.client.HTTPResponse):
    def __init__(self, sock, *args, budget, **kwargs):
        super().__init__(sock, *args, **kwargs)
        # HTTPResponse has not read yet. Replace its buffered file so reads
        # cannot prefetch application bytes outside the shared meter.
        self.fp.close()
        self.first_byte_at = None
        self.fp = MeteredReader(sock.makefile('rb', buffering=0), budget, self)

    def begin(self):
        super().begin()
        self.fp.headers = False


class TimedConnection(http.client.HTTPSConnection):
    def __init__(self, host, budget):
        if host not in HOSTS:
            raise ValueError('unsupported host')
        super().__init__(host, timeout=REQUEST_TIMEOUT, context=ssl.create_default_context())
        self.response_class = lambda *args, **kwargs: TimedResponse(*args, budget=budget, **kwargs)
        self.connect_seconds = self.tls_seconds = 0.0

    def connect(self):
        started = time.monotonic()
        http.client.HTTPConnection.connect(self)
        connected = time.monotonic()
        self.connect_seconds = connected - started
        self.sock = self._context.wrap_socket(self.sock, server_hostname=self.host)
        self.tls_seconds = time.monotonic() - connected


def cache_headers(response):
    result = {}
    age = response.getheader('Age', '')
    if re.fullmatch(r'[0-9]{1,12}', age):
        result['age'] = age
    policy = response.getheader('Cache-Control', '')
    safe = []
    for directive in policy.lower().split(','):
        directive = directive.strip()
        if re.fullmatch(r'(public|private|no-cache|no-store|must-revalidate|proxy-revalidate|immutable|no-transform)', directive):
            safe.append(directive)
        elif re.fullmatch(r'(max-age|s-maxage|stale-while-revalidate|stale-if-error)=[0-9]{1,12}', directive):
            safe.append(directive)
    if safe:
        result['cache-control'] = ', '.join(safe)[:256]
    for name in ('CF-Cache-Status', 'X-Cache'):
        value = response.getheader(name, '').strip().upper()
        if value in ('HIT', 'MISS', 'DYNAMIC', 'BYPASS', 'EXPIRED', 'REVALIDATED', 'UPDATING', 'STALE'):
            result[name.lower()] = value
    return result


def redirect_destination(response, expected_hash):
    """Describe only an exact public leaf URL; never retain raw Location text."""
    if response.status not in (301, 302, 303, 307, 308):
        return None
    hosts = '|'.join(re.escape(host) for host in HOSTS)
    match = re.fullmatch(r'https://(' + hosts + r')/' + re.escape(expected_hash) + r'(\.bin)?',
                         response.getheader('Location', ''))
    if match:
        return dict(same_hash=True, host=match.group(1), suffix=match.group(2) or 'none')
    return None


def request(connection, host, sample, sample_number, round_number, budget):
    started = time.monotonic()
    before = budget['bytes']
    connection.connect_seconds = connection.tls_seconds = 0.0
    row = dict(host=host, sample=sample_number, round=round_number,
               request_path_suffix=REQUEST_PATH_SUFFIX, redirect_destination=None,
               connection_reused=connection.sock is not None, http_code=None,
               connect_seconds=0.0, tls_seconds=0.0, ttfb_seconds=None,
               total_seconds=None, body_bytes=0, response_bytes=0,
               expected_hash_match=False, cache_headers={}, error=None)
    response = None
    try:
        with deadline():
            connection.request('GET', '/' + sample['sha256'] + REQUEST_PATH_SUFFIX,
                               headers={'Accept-Encoding': 'identity'})
            response = connection.getresponse()
            row['http_code'] = response.status
            row['ttfb_seconds'] = round(response.first_byte_at - started, 6)
            row['cache_headers'] = cache_headers(response)
            row['redirect_destination'] = redirect_destination(response, sample['sha256'])
            if response.status != 200:
                row['error'] = 'http_status'
            elif response.getheader('Content-Encoding', 'identity') != 'identity':
                row['error'] = 'content_encoding'
            else:
                digest = hashlib.sha256()
                while row['body_bytes'] <= sample['bytes']:
                    block = response.read1(min(16384, sample['bytes'] + 1 - row['body_bytes']))
                    if not block:
                        break
                    row['body_bytes'] += len(block)
                    digest.update(block)
                row['expected_hash_match'] = (row['body_bytes'] == sample['bytes'] and digest.hexdigest() == sample['sha256'])
                if not row['expected_hash_match']:
                    row['error'] = 'size_or_hash'
    except TimeoutError:
        row['error'] = 'timeout'
    except LimitExceeded:
        row['error'] = 'response_limit'
    except (OSError, http.client.HTTPException, ValueError):
        # Exception strings can contain addresses, certificate names or paths.
        row['error'] = 'transport'
    finally:
        row['connect_seconds'] = round(connection.connect_seconds, 6)
        row['tls_seconds'] = round(connection.tls_seconds, 6)
        row['total_seconds'] = round(time.monotonic() - started, 6)
        row['response_bytes'] = budget['bytes'] - before
        if row['error']:
            connection.close()
        if response:
            response.close()
    return row


def probe(samples, connection_factory=TimedConnection, rows=None):
    budget = {'bytes': 0}
    rows = [] if rows is None else rows
    for host in HOSTS:
        connection = connection_factory(host, budget)
        try:
            for round_number in range(1, ROUNDS + 1):
                for sample_number, sample in enumerate(samples, 1):
                    if budget['bytes'] >= MAX_RESPONSE_BYTES:
                        return rows, budget['bytes']
                    rows.append(request(connection, host, sample, sample_number, round_number, budget))
        finally:
            connection.close()
    return rows, budget['bytes']


def main():
    report = dict(schema='public-source-http-timing-v1', passed=False,
                  response_byte_limit=MAX_RESPONSE_BYTES, response_bytes=0,
                  request_timeout_seconds=REQUEST_TIMEOUT, rounds=ROUNDS,
                  redirects=False, retries=0, results=[])
    try:
        samples = samples_from_env(os.environ)
    except ValueError:
        report['error'] = 'invalid_inputs'
    else:
        report['samples'] = samples
        try:
            _, report['response_bytes'] = probe(samples, rows=report['results'])
            report['passed'] = len(report['results']) == 12 and all(row['expected_hash_match'] for row in report['results'])
        except Exception:
            # Preserve earlier results without emitting unexpected private data.
            report['response_bytes'] = sum(row['response_bytes'] for row in report['results'])
            report['error'] = 'internal_error'
    OUTPUT.write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
