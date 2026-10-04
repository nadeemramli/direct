"""Recovery-client checks; no live workspace or credential is used."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("remote_read", Path(__file__).parents[1] / "direct-remote-read.py")
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


class RecoveryTests(unittest.TestCase):
    def fixture(self):
        content = {"key": "DIR-1", "version": 2, "acceptance": "AC1: résumé persists", "brief": "Read current source"}
        digest = hashlib.sha256(json.dumps(content, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        context = dict(content, fingerprint=digest, observed_at=1234)
        manifest = {"session": "fixture", "issues": [{"key": "DIR-1", "version": 2, "fingerprint": digest}]}
        return manifest, context

    def run_client(self, responses):
        output = io.StringIO()
        with patch.object(client.sys, "argv", ["read", "--url", "https://bridge.example", "--proxy-credential"]), patch.object(client, "retrieve", side_effect=responses) as reader, contextlib.redirect_stdout(output):
            client.main()
        # Proxy mode never supplies a secret to its HTTP client.
        for call in reader.call_args_list:
            self.assertIsNone(call.args[2])
        return json.loads(output.getvalue())

    def test_proxy_receipt_checks_unicode_hash_and_manifest_reread(self):
        manifest, context = self.fixture()
        result = self.run_client([manifest, context, manifest])
        self.assertEqual(result["receipt"][0]["fingerprint"], context["fingerprint"])
        self.assertNotIn("token", json.dumps(result))

    def test_version_drift_and_content_tampering_prevent_receipt(self):
        manifest, context = self.fixture()
        changed = dict(context, brief="tampered")
        with self.assertRaises(ValueError):
            self.run_client([manifest, changed, manifest])
        later = {"issues": [dict(manifest["issues"][0], version=3)]}
        with self.assertRaises(ValueError):
            self.run_client([manifest, context, later])

    def test_credentials_in_urls_and_plain_http_are_rejected(self):
        for url in ["http://bridge.example", "https://secret@bridge.example", "https://bridge.example?token=secret", "https://bridge.example/path"]:
            with patch.object(client.sys, "argv", ["read", "--url", url, "--proxy-credential"]), self.assertRaises(ValueError):
                client.main()


if __name__ == "__main__":
    unittest.main()
