#!/usr/bin/env python3
"""Download retry tests for fetch-media-tools.py, with a stand-in network."""

from __future__ import annotations

import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error


SPEC = importlib.util.spec_from_file_location("neptune_video_fetch_media_tools", Path(__file__).with_name("fetch-media-tools.py"))
FETCHER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FETCHER)
URL = "https://example.invalid/ffmpeg.tar.xz"
BODY = b"a pinned build"
DIGEST = hashlib.sha256(BODY).hexdigest()


class Network:
    """Answers with each of `answers` in turn: bytes, or an error to raise."""

    def __init__(self, *answers):
        self.answers = list(answers)
        self.asked = 0

    def __call__(self, request, timeout):
        self.asked += 1
        answer = self.answers.pop(0)
        if isinstance(answer, BaseException):
            raise answer
        return answer


class Dropping(io.BytesIO):
    """A response whose connection is reset part way through."""

    def read(self, size=-1):
        if self.tell():
            raise ConnectionResetError("reset by peer")
        return super().read(4)


class DownloadTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="neptune-video-fetch-")
        self.addCleanup(temporary.cleanup)
        self.archive = Path(temporary.name) / "0-ffmpeg.tar.xz"

    def download(self, network, attempts=3):
        with patch.object(FETCHER.urllib.request, "urlopen", network), patch("sys.stderr", io.StringIO()):
            FETCHER.download(URL, DIGEST, self.archive, attempts=attempts, pause=0)

    def test_a_timeout_and_a_dropped_connection_are_tried_again(self):
        network = Network(urllib.error.URLError(TimeoutError("timed out")), Dropping(BODY), io.BytesIO(BODY))
        self.download(network)
        self.assertEqual(network.asked, 3)
        self.assertEqual(self.archive.read_bytes(), BODY)

    def test_the_last_failure_is_raised_and_leaves_no_file(self):
        network = Network(TimeoutError("timed out"), TimeoutError("timed out again"))
        with self.assertRaisesRegex(TimeoutError, "again"):
            self.download(network, attempts=2)
        self.assertFalse(self.archive.exists())

    def test_a_wrong_digest_is_not_tried_again(self):
        network = Network(io.BytesIO(b"something else"), io.BytesIO(BODY))
        with self.assertRaisesRegex(ValueError, "expected"):
            self.download(network)
        self.assertEqual(network.asked, 1)

    def test_a_file_already_there_is_neither_replaced_nor_removed(self):
        self.archive.write_bytes(b"kept")
        network = Network(io.BytesIO(BODY))
        with self.assertRaises(FileExistsError):
            self.download(network)
        self.assertEqual(network.asked, 0)
        self.assertEqual(self.archive.read_bytes(), b"kept")


if __name__ == "__main__":
    unittest.main()
