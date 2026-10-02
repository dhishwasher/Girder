import hashlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock

from tools import prepare_typescript_audit_extension as subject


class Response(io.BytesIO):
    status = 200
    headers = {}

    def geturl(self):
        return 'https://codeload.github.com/example/repo/tar.gz/refs/tags/v1'


class AcquisitionTests(unittest.TestCase):
    def test_missing_length_does_not_skip_actual_size_or_hash_verification(self):
        expected = b'pinned archive bytes'
        artifact = {'url': Response(b'').geturl(), 'bytes': len(expected),
                    'sha256': hashlib.sha256(expected).hexdigest()}
        for body, success in [(expected, True), (expected[:-1], False),
                              (expected+b'x', False), (b'x'*len(expected), False)]:
            with self.subTest(body=body), tempfile.TemporaryDirectory() as directory:
                cache = Path(directory)
                opener = Mock()
                opener.open.return_value = Response(body)
                with patch.object(subject, 'CACHE', cache), patch.object(subject.urllib.request, 'build_opener', return_value=opener):
                    if success:
                        self.assertEqual(subject.acquire_tag(artifact).read_bytes(), expected)
                    else:
                        with self.assertRaises(RuntimeError):
                            subject.acquire_tag(artifact)
                        self.assertEqual(list(cache.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
