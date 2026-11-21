import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import record_session as recorder


class RecorderTests(unittest.TestCase):
    def test_backend_freeze_keeps_heartbeat(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "capture"
            with patch.object(recorder, "request", side_effect=TimeoutError("backend unresponsive")):
                result = recorder.main(["--port", "15759", "--output", str(output),
                                        "--interval", "0.05", "--duration", "1"])
            records = [json.loads(line) for line in (output / "recorder.jsonl").read_text().splitlines()]
            self.assertEqual(result, 0)
            self.assertTrue(any(row["type"] == "endpoint_error" for row in records))
            self.assertTrue(any(row["type"] == "heartbeat" for row in records))
            self.assertEqual(records[-1]["type"], "stop")

    def test_duplicates_and_app_restart(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "events.jsonl"
            journal = recorder.Journal(path)
            collector = recorder.EventCollector(journal)
            event = {"id": 1, "timestamp": "first", "payload_snippet": "full payload"}
            collector.accept([event, event])
            collector.accept([dict(event, timestamp="after restart")])
            journal.close()
            self.assertEqual(len(path.read_text().splitlines()), 2)
            self.assertEqual(collector.count, 2)

    def test_log_truncation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source"
            target = root / "target"
            source.mkdir()
            target.mkdir()
            journal = recorder.Journal(root / "recorder.jsonl")
            log = source / "launch.log"
            log.write_bytes(b"original\n")
            offsets = {}
            recorder.copy_log_updates([source], target, offsets, journal)
            log.write_bytes(b"new\n")
            recorder.copy_log_updates([source], target, offsets, journal)
            journal.close()
            self.assertEqual((target / "0-launch.log").read_bytes(), b"original\nnew\n")

    def test_malformed_endpoint(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"not-json")

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with self.assertRaises(ValueError):
                recorder.request(server.server_port, "/network/events", 1)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_live_capture(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(json.dumps([{"id": 7, "timestamp": "test", "payload_snippet": "complete"}]).encode())

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as temp:
                output = Path(temp) / "capture"
                result = recorder.main(["--port", str(server.server_port), "--output", str(output),
                                        "--interval", "0.05", "--duration", "1"])
                self.assertEqual(result, 0)
                events = [json.loads(line) for line in (output / "network-events.jsonl").read_text().splitlines()]
                self.assertEqual(len(events), 1)
                self.assertEqual(events[0]["payload_snippet"], "complete")
                records = [json.loads(line) for line in (output / "recorder.jsonl").read_text().splitlines()]
                self.assertTrue(any(row["type"] == "heartbeat" for row in records))
                self.assertEqual(records[-1]["type"], "stop")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
