import copy
import hashlib
import io
import json
import time
import unittest
from unittest.mock import patch

import zapstore_publish as publish


def signed(event):
    result = copy.deepcopy(event)
    result['id'] = hashlib.sha256(json.dumps(result, sort_keys=True).encode()).hexdigest()
    result['sig'] = 'fixture-signature'
    return result


class FakeRelay:
    """Relay with independent acknowledgement, pending and public state."""
    def __init__(self, conditional=False, proof=True):
        self.public = {}
        self.pending = {}
        self.acks = []
        self.signed = []
        self.conditional = conditional
        self.proofs = [signed({'kind': 30509, 'pubkey': publish.AUTHOR, 'created_at': 1,
                              'tags': [['d', 'c' * 64], ['expiry', str(int(time.time()) + 3600)],
                                       ['signature', 'fixture-certificate-signature']], 'content': ''})] if proof else []

    def verify(self, event):
        if event['sig'] != 'fixture-signature':
            raise ValueError('signature failed')

    def sign(self, event):
        self.signed.append(copy.deepcopy(event))
        return signed(event)

    def query(self, arguments):
        if '--kind' in arguments:
            return copy.deepcopy(self.proofs)
        wanted = arguments[1::2]
        return [copy.deepcopy(self.public[event_id]) for event_id in wanted if event_id in self.public]

    def publish(self, events):
        for event in events:
            reference = [t[1] for t in event['tags'] if t[0] == 'a']
            held = event['kind'] == 30063 and (self.conditional or not reference)
            self.acks.append(['OK', event['id'], True, 'the listing will be saved when it is complete' if held else ''])
            (self.pending if held else self.public)[event['id']] = copy.deepcopy(event)


class ZapstorePublicationTests(unittest.TestCase):
    def setUp(self):
        self.expected = {'version': '2026.10.24.1', 'version_code': '2026102401',
                         'name': 'iris-chat-v2026.10.24.1-android-arm64.apk',
                         'size': 3, 'sha256': hashlib.sha256(b'apk').hexdigest()}
        self.apk = signed({'kind': 3063, 'pubkey': publish.AUTHOR, 'created_at': 1, 'content': '',
            'tags': [['i', publish.PACKAGE], ['version', self.expected['version']],
                     ['version_code', self.expected['version_code']], ['filename', self.expected['name']],
                     ['x', self.expected['sha256']], ['size', '3'], ['url', publish.CDN + '/' + self.expected['sha256']],
                     ['m', 'application/vnd.android.package-archive'], ['f', 'android-arm64-v8a'], ['apk_certificate_hash', 'c' * 64]]})
        self.release = signed({'kind': 30063, 'pubkey': publish.AUTHOR, 'created_at': 1, 'content': 'Notes',
            'tags': [['i', publish.PACKAGE], ['version', self.expected['version']],
                     ['d', publish.PACKAGE + '@' + self.expected['version']], ['c', 'main'],
                     ['f', 'android-arm64-v8a'], ['e', self.apk['id'], publish.RELAY]]})
        self.app = signed({'kind': 32267, 'pubkey': publish.AUTHOR, 'created_at': 1, 'content': 'Iris',
                           'tags': [['d', publish.PACKAGE], ['name', 'Iris Chat']]})
        self.captured = [self.app, self.release, self.apk]

    def read_apk(self, expected):
        self.assertEqual(expected, self.expected)
        return {'bytes': 3, 'sha256': expected['sha256']}

    def test_legacy_publisher_missing_app_reference_is_corrected_before_send(self):
        relay = FakeRelay()
        result = publish.complete_publication(relay, self.captured, self.expected, self.read_apk)
        self.assertTrue(result['passed'])
        self.assertEqual(len(relay.signed), 1)
        corrected = relay.signed[0]
        self.assertEqual(corrected['tags'], [['a', '32267:' + publish.AUTHOR + ':' + publish.PACKAGE], *self.release['tags']])
        self.assertEqual(corrected['content'], self.release['content'])
        self.assertEqual(relay.public[self.apk['id']], self.apk)
        self.assertEqual(relay.public[self.app['id']], self.app)
        self.assertNotIn('a', [tag[0] for tag in self.release['tags']])

    def test_true_conditional_ack_without_public_release_is_failure(self):
        relay = FakeRelay(conditional=True)
        with self.assertRaisesRegex(ValueError, 'complete release is not public'):
            publish.complete_publication(relay, self.captured, self.expected,
                                         lambda _: self.fail('must not accept/download incomplete listing'))
        self.assertTrue(all(ack[2] is True for ack in relay.acks))
        self.assertEqual(len(relay.pending), 1)

    def test_repeat_publication_checks_public_state_and_preserves_app_and_apk(self):
        relay = FakeRelay()
        for _ in range(2):
            result = publish.complete_publication(relay, self.captured, self.expected, self.read_apk)
            self.assertTrue(result['passed'])
            self.assertEqual(relay.public[self.apk['id']], self.apk)
            self.assertEqual(relay.public[self.app['id']], self.app)
            self.assertEqual(result['event_ids'][0], self.apk['id'])
            self.assertEqual(result['event_ids'][2], self.app['id'])
        self.assertEqual(len(relay.acks), 6)

    def test_existing_correct_app_reference_is_not_resigned(self):
        event = {k: v for k, v in self.release.items() if k not in ('id', 'sig')}
        event['tags'] = [['a', '32267:' + publish.AUTHOR + ':' + publish.PACKAGE], *event['tags']]
        relay = FakeRelay()
        publish.complete_publication(relay, [self.app, signed(event), self.apk], self.expected, self.read_apk)
        self.assertEqual(relay.signed, [])

    def test_missing_or_expired_c1_does_not_send(self):
        for relay in (FakeRelay(proof=False), FakeRelay()):
            if relay.proofs:
                relay.proofs[0]['tags'][1][1] = '1'
            with self.subTest(proof=bool(relay.proofs)), self.assertRaisesRegex(ValueError, 'proof'):
                publish.complete_publication(relay, self.captured, self.expected, self.read_apk)
            self.assertEqual(relay.acks, [])

    def test_newer_revocation_cannot_fall_back_to_old_valid_c1(self):
        relay = FakeRelay()
        revoked = copy.deepcopy(relay.proofs[0])
        revoked['created_at'] = 2
        revoked['tags'] = [['d', 'c' * 64], ['revoked', 'true']]
        relay.proofs.append(revoked)
        with self.assertRaisesRegex(ValueError, 'revoked'):
            publish.complete_publication(relay, self.captured, self.expected, self.read_apk)
        self.assertEqual(relay.acks, [])

    def test_public_apk_body_must_match_size_and_hash_not_merely_http_success(self):
        class Response(io.BytesIO):
            status = 200
        for body, error in ((b'ap', 'differs from immutable release'),
                            (b'bad', 'differs from immutable release'),
                            (b'apkx', 'exceeds expected size')):
            with self.subTest(body=body), patch.object(publish, 'build_opener') as opener:
                opener.return_value.open.return_value = Response(body)
                with self.assertRaisesRegex(ValueError, error):
                    publish.verify_cdn(self.expected)

    def test_public_apk_redirect_allows_only_the_exact_cdn_blob(self):
        class Response(io.BytesIO):
            status = 200
        original = publish.CDN + '/' + self.expected['sha256']
        destination = 'https://zsapk.b-cdn.net/blobs/' + self.expected['sha256'] + '.apk'

        def server_for(source, target):
            def create(*handlers):
                class Server:
                    def open(self, request, timeout):
                        redirected = handlers[1].redirect_request(publish.Request(source), None, 302, 'Found', {}, target)
                        self_test.assertEqual(redirected.full_url, destination)
                        return Response(b'apk')
                return Server()
            return create

        self_test = self
        with patch.object(publish, 'build_opener', side_effect=server_for(original, destination)):
            self.assertEqual(publish.verify_cdn(self.expected), {'bytes': 3, 'sha256': self.expected['sha256']})
        for source, target in ((original, 'https://unrelated.invalid/apk'),
                               (original, destination.replace('https:', 'http:')),
                               (original, destination.replace(self.expected['sha256'], '0' * 64)),
                               (destination, destination)):
            with self.subTest(source=source, target=target):
                with patch.object(publish, 'build_opener', side_effect=server_for(source, target)):
                    with self.assertRaisesRegex(ValueError, 'Unexpected APK redirect'):
                        publish.verify_cdn(self.expected)

    def test_public_apk_streaming_deadline_is_enforced(self):
        class Response(io.BytesIO):
            status = 200
        with patch.object(publish, 'build_opener') as opener, patch.object(publish.time, 'monotonic', side_effect=(0, 91)):
            opener.return_value.open.return_value = Response(b'apk')
            with self.assertRaisesRegex(ValueError, 'deadline exceeded'):
                publish.verify_cdn(self.expected)


if __name__ == '__main__':
    unittest.main()
