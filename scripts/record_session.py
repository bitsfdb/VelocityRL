#!/usr/bin/env python3
"""Record VelocityRL diagnostics without its UI. Python 3.9+, standard library only.

Run before reproducing a problem; stop with Ctrl+C. Optional Wireshark dumpcap
capture is independent of the app and preserves encrypted packets as-is.
"""
import argparse
import collections
import datetime as dt
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import urllib.request


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


class Journal:
    def __init__(self, path):
        self.file = path.open("a", encoding="utf-8", buffering=1)
        self.lock = threading.Lock()

    def write(self, record):
        with self.lock:
            self.file.write(json.dumps(record, ensure_ascii=True) + "\n")
            self.file.flush()

    def close(self):
        self.file.close()


class EventCollector:
    def __init__(self, journal):
        self.journal = journal
        self.keys = set()
        self.order = collections.deque()
        self.count = 0

    def accept(self, events):
        if not isinstance(events, list):
            raise ValueError("Network endpoint did not return an event array")
        for event in events:
            if not isinstance(event, dict) or "id" not in event or "timestamp" not in event:
                raise ValueError("Invalid network event")
            key = (event["id"], event["timestamp"])
            if key in self.keys:
                continue
            self.journal.write(event)
            self.keys.add(key)
            self.order.append(key)
            self.count += 1
            if len(self.order) > 5000:
                self.keys.discard(self.order.popleft())


def request(port, path, timeout):
    # Never use system proxy settings for the local diagnostic endpoint.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(f"http://127.0.0.1:{port}{path}", timeout=timeout) as response:
        return json.load(response)


def log_roots():
    roots = []
    for variable in ("LOCALAPPDATA", "APPDATA"):
        if os.environ.get(variable):
            for name in ("com.velocityrl.app", "VelocityRL"):
                base = Path(os.environ[variable]) / name
                roots.extend((base / "logs", base))
    return list(dict.fromkeys(roots))


def discover_ports(explicit, roots):
    if explicit:
        return [explicit]
    ports = [15759, 15758]
    import re
    for root in roots:
        for path in root.glob("*launch*.log"):
            try:
                with path.open("rb") as source:
                    source.seek(max(0, path.stat().st_size - 131072))
                    tail = source.read().decode("utf-8", errors="replace")
                found = re.findall(r"(?:broker.*?127\.0\.0\.1:)(\d{1,5})", tail)
                ports.extend(int(p) for p in reversed(found) if 0 < int(p) < 65536)
            except OSError:
                pass
    return list(dict.fromkeys(ports))


def poll_events(stop, args, collector, diagnostics, roots):
    active = None
    last_discovery = 0
    candidates = []
    while not stop.is_set():
        started = time.monotonic()
        if started - last_discovery > 5 or not candidates:
            candidates = discover_ports(args.port, roots)
            last_discovery = started
        ports = [active] if active else candidates
        connected = False
        for port in ports:
            if stop.is_set():
                break
            try:
                collector.accept(request(port, "/network/events", args.timeout))
                if active != port:
                    diagnostics.write({"time": now(), "type": "connected", "port": port})
                    print(f"Connected to VelocityRL on port {port}.", flush=True)
                active = port
                connected = True
                break
            except Exception as error:
                diagnostics.write({"time": now(), "type": "endpoint_error", "port": port,
                                   "error": str(error)})
        if not connected:
            active = None
        stop.wait(max(0.05, args.interval - (time.monotonic() - started)))


def copy_log_updates(roots, destination, offsets, diagnostics):
    for index, root in enumerate(roots):
        for source in root.glob("*.log"):
            try:
                stat = source.stat()
                previous = offsets.get(source)
                identity = (stat.st_dev, stat.st_ino)
                offset = previous[1] if previous and previous[0] == identity else 0
                if stat.st_size < offset:
                    offset = 0
                with source.open("rb") as incoming:
                    incoming.seek(offset)
                    data = incoming.read(1024 * 1024)
                if data:
                    with (destination / f"{index}-{source.name}").open("ab") as output:
                        output.write(data)
                offsets[source] = (identity, offset + len(data))
            except OSError as error:
                diagnostics.write({"time": now(), "type": "log_copy_error",
                                   "file": str(source), "error": str(error)})


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, help="Override automatic local broker discovery")
    parser.add_argument("--output", type=Path, help="New output folder")
    parser.add_argument("--interval", type=float, default=1.0)
    parser.add_argument("--timeout", type=float, default=1.5)
    parser.add_argument("--duration", type=float, help="Stop automatically after this many seconds")
    parser.add_argument("--packet-interface", action="append", default=[],
                        help="Also run Wireshark dumpcap on this interface ID; repeat for multiple interfaces")
    args = parser.parse_args(argv)
    if args.interval <= 0 or args.timeout <= 0 or (args.duration is not None and args.duration <= 0):
        parser.error("Intervals, timeout and duration must be positive")
    if args.port is not None and not 0 < args.port < 65536:
        parser.error("Port must be between 1 and 65535")
    folder = args.output or Path(__file__).resolve().parents[1] / "recordings" / dt.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    folder.mkdir(parents=True, exist_ok=False)
    logs = folder / "app-logs"
    logs.mkdir()
    diagnostics = Journal(folder / "recorder.jsonl")
    network = Journal(folder / "network-events.jsonl")
    collector = EventCollector(network)
    stop = threading.Event()
    roots = log_roots()
    offsets = {}
    packet_process = None
    packet_log = None
    worker = None
    print(f"Recording to: {folder.resolve()}\nKeep this window open. Stop with Ctrl+C.", flush=True)
    print("Waiting for VelocityRL. No UI export is needed.", flush=True)
    (folder / "README.txt").write_text(
        "network-events.jsonl: complete events exposed by VelocityRL, saved continuously.\n"
        "recorder.jsonl: heartbeat, connection failures and collection errors.\n"
        "app-logs/: copies of available app logs.\n"
        "The app endpoint exposes processed traffic; originals are not available when overwritten.\n"
        "A backend hang prevents new decoded events. Heartbeats continue independently.\n"
        "The app ring retains 500 events; traffic faster than polling can cause gaps.\n"
        "Raw packets are recorded only with --packet-interface and installed dumpcap.\n"
        "Encrypted packets remain encrypted. Captures may contain login/session data; keep private.\n",
        encoding="utf-8")
    diagnostics.write({"time": now(), "type": "start", "log_roots": [str(p) for p in roots]})
    try:
        if args.packet_interface:
            dumpcap = shutil.which("dumpcap") or str(Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Wireshark/dumpcap.exe")
            command = [dumpcap]
            for interface in args.packet_interface:
                command.extend(["-i", interface])
            command.extend(["-w", str(folder.resolve() / "packets.pcapng"),
                            "-b", "filesize:32768", "-b", "files:8"])
            packet_log = (folder / "dumpcap.log").open("wb")
            packet_process = subprocess.Popen(command, stdout=packet_log, stderr=packet_log,
                                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        worker = threading.Thread(target=poll_events, args=(stop, args, collector, diagnostics, roots), daemon=True)
        worker.start()
        started = time.monotonic()
        next_message = started
        while args.duration is None or time.monotonic() - started < args.duration:
            copy_log_updates(roots, logs, offsets, diagnostics)
            diagnostics.write({"time": now(), "type": "heartbeat", "events_saved": collector.count,
                               "poller_alive": worker.is_alive(),
                               "packet_exit": packet_process.poll() if packet_process else None})
            if time.monotonic() >= next_message:
                print(f"{dt.datetime.now():%H:%M:%S}  Saved {collector.count} network events. Recorder active.", flush=True)
                next_message = time.monotonic() + 10
            if not worker.is_alive():
                raise RuntimeError("Network poller stopped; check recorder.jsonl")
            if packet_process and packet_process.poll() is not None:
                raise RuntimeError("Packet capture stopped; check dumpcap.log (capture permissions may be required)")
            stop.wait(1)
    except KeyboardInterrupt:
        pass
    except Exception as error:
        diagnostics.write({"time": now(), "type": "fatal", "error": str(error)})
        print(f"Recorder error: {error}", flush=True)
        return 1
    finally:
        stop.set()
        if worker:
            worker.join(timeout=args.timeout * 2 + 2)
        if packet_process and packet_process.poll() is None:
            packet_process.terminate()
            try:
                packet_process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                packet_process.kill()
                packet_process.wait()
        if packet_log:
            packet_log.close()
        diagnostics.write({"time": now(), "type": "stop", "events_saved": collector.count})
        if not worker or not worker.is_alive():
            network.close()
            diagnostics.close()
        print(f"Saved capture: {folder.resolve()}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
