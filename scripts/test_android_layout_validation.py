import hashlib
import io
import unittest
import zipfile
from android_layout_validation import payload


class ApkContentIntegrityTests(unittest.TestCase):
    def test_service_loader_and_version_resources_are_not_signatures(self):
        entries = {'classes.dex': b'bytecode', 'resources.arsc': b'resources',
            'META-INF/services/kotlinx.coroutines.CoroutineExceptionHandler': b'handler',
            'META-INF/androidx.core_core.version': b'1',
            'META-INF/MANIFEST.MF': b'old signature', 'META-INF/CERT.SF': b'old signature',
            'META-INF/CERT.RSA': b'old signature', 'lib/arm64-v8a/libcore.so': b'arm64'}
        data = io.BytesIO()
        with zipfile.ZipFile(data, 'w') as archive:
            for name, value in entries.items(): archive.writestr(name, value)
        with zipfile.ZipFile(data) as archive: actual = payload(archive)
        expected = ['classes.dex', 'resources.arsc',
            'META-INF/services/kotlinx.coroutines.CoroutineExceptionHandler', 'META-INF/androidx.core_core.version']
        self.assertEqual(set(expected), set(actual))
        for name in expected: self.assertEqual(hashlib.sha256(entries[name]).hexdigest(), actual[name])


if __name__ == '__main__': unittest.main()
