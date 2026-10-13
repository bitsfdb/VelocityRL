#!/usr/bin/env python3
"""
VelocityRL Traffic Inspector & Live Traffic Recorder
Inspect ALL traffic, client headers, WebSocket frames, RPC payloads, and track loadouts across all apps.
Pure Python 3 Standard Library - Zero external dependencies.

Usage:
    python traffic_inspector.py [PATH_OR_FILE] [OPTIONS]

Display Modes:
    --all, -a            Dump ALL traffic frames with all client/server headers and complete payloads (no truncation)
    --client-only, -c    Dump only outbound client traffic (all client headers & payloads from apps)
    --server-only, -s    Dump only inbound server responses
    --headers            Print all HTTP & PsyNet headers for all packets
    --seq SEQ            Inspect a specific sequence number or range (e.g. --seq 97 or --seq 90-105)
    --filter KEYWORD     Search packets by keyword across headers, paths, services, and payloads (untruncated)
    --service SERVICE    Filter packets by PsyService name
    --summary            Show high-level breakdown of services, endpoints, and connected match servers
    --loadouts           Show loadout history table highlighting car body changes & reversions
    --timeline           Show chronological match lifecycle timeline (reservations, goals, spawns)

Live Operations:
    -f, --follow         Live follow/tail packets as they are logged
    --listen [PORT]      Start a local HTTP proxy recorder (default port 8888) to capture & log ALL traffic
                         and client headers from ANY application on the system into rpc-probe.jsonl
"""

import sys
import os
import json
import time
import argparse
import socket
from datetime import datetime
from http.server import HTTPServer, BaseHTTPRequestHandler
import urllib.request
import urllib.parse
import threading

class Colors:
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    RED = "\033[91m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    BLUE = "\033[94m"
    MAGENTA = "\033[95m"
    CYAN = "\033[96m"
    GRAY = "\033[90m"
    BG_BLUE = "\033[44m"
    BG_MAGENTA = "\033[45m"

KNOWN_CARS = {
    23: "Octane",
    403: "Dominus",
    4284: "Fennec",
    11603: "Fennec ZR-F",
    532: "Breakout",
    803: "Titanium White Octane",
}

def format_product_id(pid):
    if pid in KNOWN_CARS:
        return f"{pid} ({KNOWN_CARS[pid]})"
    return str(pid)

def find_target_file(path_arg):
    if path_arg and os.path.isfile(path_arg):
        return os.path.abspath(path_arg)
    
    candidates = []
    if path_arg and os.path.isdir(path_arg):
        candidates.extend([
            os.path.join(path_arg, "rpc-probe.jsonl"),
            os.path.join(path_arg, "logs", "rpc-probe.jsonl"),
        ])
    
    candidates.extend([
        "./rpc-probe.jsonl",
        "./logs/rpc-probe.jsonl",
        "/root/diag_extracted/logs/rpc-probe.jsonl",
        os.path.expandvars(r"%APPDATA%\com.velocityrl.app\logs\rpc-probe.jsonl"),
        os.path.expandvars(r"%APPDATA%\VelocityRL\logs\rpc-probe.jsonl"),
        os.path.expanduser("~/.config/VelocityRL/logs/rpc-probe.jsonl"),
    ])

    for c in candidates:
        if os.path.isfile(c):
            return os.path.abspath(c)
    return None

def parse_inner_payload(body_raw):
    """Attempt to parse outer and nested stringified JSON payloads (e.g. MessagePayload, EventData)."""
    if not body_raw:
        return None, None
    try:
        outer = json.loads(body_raw)
    except Exception:
        return None, None

    inner = None
    if isinstance(outer, dict):
        for key in ["MessagePayload", "EventData", "Data", "Payload"]:
            if key in outer and isinstance(outer[key], str):
                try:
                    inner = json.loads(outer[key])
                    break
                except Exception:
                    pass
    return outer, inner

def extract_loadouts(outer, inner):
    """Extract player loadout lists from various PsyNet and DSR schemas."""
    found = []

    def scan_dict(d):
        if not isinstance(d, dict):
            return
        if "Players" in d and isinstance(d["Players"], list):
            for p in d["Players"]:
                if isinstance(p, dict) and "Loadout" in p:
                    found.append((p.get("PlayerID", "local"), p.get("PlayerName", "Unknown"), p["Loadout"]))
        if "Loadout" in d:
            found.append(("local", "LoadoutNode", d["Loadout"]))
        if "Products" in d:
            found.append(("local", "ProductsNode", d["Products"]))

    if isinstance(inner, dict):
        scan_dict(inner)
    if isinstance(outer, dict):
        scan_dict(outer)
    return found

def format_loadout_display(loadout):
    if isinstance(loadout, list):
        items = []
        for i, val in enumerate(loadout):
            slot_name = "Slot" + str(i)
            if i == 0:
                slot_name = "Body"
            elif i == 1 and len(loadout) >= 6:
                slot_name = "Decal"
            elif (i == 1 and len(loadout) == 5) or (i == 2 and len(loadout) >= 6):
                slot_name = "Wheels"
            elif (i == 2 and len(loadout) == 5) or (i == 3 and len(loadout) >= 6):
                slot_name = "Boost"
            elif (i == 3 and len(loadout) == 5) or (i == 4 and len(loadout) >= 6):
                slot_name = "Trail"
            elif (i == 4 and len(loadout) == 5) or (i == 5 and len(loadout) >= 6):
                slot_name = "GoalExp"
            items.append(f"{slot_name}={format_product_id(val)}")
        return " [ " + ", ".join(items) + " ]"
    elif isinstance(loadout, dict):
        if "Products" in loadout and isinstance(loadout["Products"], list):
            prods = []
            for p in loadout["Products"]:
                if isinstance(p, dict):
                    pid = p.get("ProductID") or p.get("ID") or p.get("ProductId")
                    paint = p.get("PaintID", 0)
                    paint_str = f" (Paint {paint})" if paint else ""
                    prods.append(f"{format_product_id(pid)}{paint_str}")
                else:
                    prods.append(format_product_id(p))
            return " { Products: [" + ", ".join(prods) + "] }"
        return str(loadout)
    return str(loadout)

def format_pretty_body(body_raw):
    """Return fully formatted, un-truncated string of body with inner JSON expanded if possible."""
    if not body_raw:
        return "(empty)"
    try:
        data = json.loads(body_raw)
        # Check if MessagePayload is an embedded JSON string, unpack it for beauty
        if isinstance(data, dict):
            for key in ["MessagePayload", "EventData", "Data", "Payload"]:
                if key in data and isinstance(data[key], str):
                    try:
                        data[key] = json.loads(data[key])
                    except Exception:
                        pass
        return json.dumps(data, indent=2, ensure_ascii=False)
    except Exception:
        # Check if it looks like URL encoded form
        if "=" in body_raw and "&" in body_raw:
            try:
                parsed = urllib.parse.parse_qs(body_raw)
                return json.dumps(parsed, indent=2)
            except Exception:
                pass
        return body_raw

def print_frame(frame, line_num=None, show_body=True, show_headers=True):
    kind = frame.get("kind", "unknown")
    seq = frame.get("seq", 0)
    ts = frame.get("ts", "")
    host = frame.get("host", "")
    path = frame.get("path", "")
    method = frame.get("method", "")
    service = frame.get("service", "")
    headers = frame.get("headers", [])
    body_raw = frame.get("body", "")

    is_c2u = "c2u" in kind or "req" in kind
    badge_color = Colors.GREEN if is_c2u else Colors.BLUE
    dir_name = "CLIENT -> SERVER" if is_c2u else "SERVER -> CLIENT"
    
    endpoint = f"{method} {host}{path}" if method or path else f"{host} ({service or kind})"
    
    sep = "━" * 80
    print(f"\n{badge_color}{Colors.BOLD}{sep}{Colors.RESET}")
    print(f"{badge_color}{Colors.BOLD} [FRAME #{seq}] {ts} | {dir_name} [{kind.upper()}]{Colors.RESET}")
    print(f" {Colors.CYAN}Endpoint / Service:{Colors.RESET} {Colors.BOLD}{endpoint}{Colors.RESET}")

    # Headers
    if show_headers and headers:
        print(f" {Colors.YELLOW}Headers ({len(headers)}):{Colors.RESET}")
        for h in headers:
            if ":" in h:
                k, v = h.split(":", 1)
                print(f"   {Colors.GRAY}{k.strip()}:{Colors.RESET} {v.strip()}")
            else:
                print(f"   {h}")

    # Loadout detection
    outer, inner = parse_inner_payload(body_raw)
    loadouts = extract_loadouts(outer, inner)
    if loadouts:
        print(f" {Colors.MAGENTA}Loadouts Detected:{Colors.RESET}")
        for pid, pname, ldata in loadouts:
            print(f"   {Colors.BOLD}{pname} ({pid}):{Colors.RESET}{format_loadout_display(ldata)}")

    # Full Payload Body (zero truncation)
    if show_body and body_raw:
        pretty = format_pretty_body(body_raw)
        print(f" {Colors.YELLOW}Payload ({len(body_raw)} bytes):{Colors.RESET}")
        for line in pretty.splitlines():
            print(f"   {line}")
    elif show_body and not body_raw:
        print(f" {Colors.GRAY}Payload: (none){Colors.RESET}")

def run_inspector(file_path, args):
    print(f"{Colors.BOLD}{Colors.CYAN}=== VelocityRL Traffic Inspector ==={Colors.RESET}")
    print(f"Reading: {Colors.YELLOW}{file_path}{Colors.RESET}")

    services = {}
    servers = set()
    loadout_history = []
    events = []
    total_frames = 0
    c2u_count = 0
    u2c_count = 0

    seq_min = None
    seq_max = None
    if args.seq:
        if "-" in args.seq:
            p = args.seq.split("-", 1)
            seq_min, seq_max = int(p[0]), int(p[1])
        else:
            seq_min = seq_max = int(args.seq)

    # If --all, --client-only, --server-only, --headers, --filter, --service, or --seq are supplied, dump frames!
    should_dump_frames = args.all or args.client_only or args.server_only or args.headers or args.filter or args.service or (args.seq is not None)

    with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
        for line_num, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                frame = json.loads(line)
            except Exception:
                continue

            total_frames += 1
            kind = frame.get("kind", "")
            svc = frame.get("service", "") or "Generic/Unknown"
            ts = frame.get("ts", "")
            seq = frame.get("seq", 0)
            body_raw = frame.get("body", "")

            if "c2u" in kind or "req" in kind:
                c2u_count += 1
            elif "u2c" in kind or "resp" in kind:
                u2c_count += 1

            services[svc] = services.get(svc, 0) + 1

            outer, inner = parse_inner_payload(body_raw)
            if outer and isinstance(outer, dict):
                addr = outer.get("ServerAddress")
                if addr:
                    servers.add(addr)
            if inner and isinstance(inner, dict):
                addr = inner.get("ServerAddress")
                if addr:
                    servers.add(addr)

            found_loadouts = extract_loadouts(outer, inner)
            for pid, pname, ldata in found_loadouts:
                loadout_history.append({
                    "line": line_num,
                    "seq": seq,
                    "ts": ts,
                    "kind": kind,
                    "service": svc,
                    "pid": pid,
                    "pname": pname,
                    "loadout": ldata,
                })

            is_reservation = "RelayToServer" in svc or "AddReservation" in body_raw or "ReservationsReady" in body_raw
            if is_reservation:
                msg_type = (outer.get("MessageType") if isinstance(outer, dict) else "") or svc
                events.append({
                    "line": line_num,
                    "seq": seq,
                    "ts": ts,
                    "kind": kind,
                    "type": "RESERVATION",
                    "label": f"[{kind}] {msg_type}",
                    "loadouts": found_loadouts,
                })

            if should_dump_frames:
                # Direction filter
                if args.client_only and not ("c2u" in kind or "req" in kind):
                    continue
                if args.server_only and not ("u2c" in kind or "resp" in kind):
                    continue

                # Sequence filter
                if seq_min is not None and not (seq_min <= seq <= seq_max):
                    continue

                # Service filter
                if args.service and args.service.lower() not in svc.lower():
                    continue

                # Keyword filter (searches line, headers, and body)
                if args.filter:
                    kw = args.filter.lower()
                    text_to_search = line.lower()
                    if kw not in text_to_search:
                        continue

                # Dump this frame completely
                print_frame(frame, line_num=line_num, show_body=not args.headers_only, show_headers=True)

    # Summary
    if args.summary or (not should_dump_frames and not args.loadouts and not args.timeline):
        print(f"\n{Colors.BOLD}--- TRAFFIC SUMMARY ---{Colors.RESET}")
        print(f"Total Frames: {total_frames} (Client Traffic: {c2u_count}, Server Traffic: {u2c_count})")
        print(f"Dedicated Match Servers Seen: {len(servers)}")
        for s in list(servers)[:8]:
            print(f"  * {s}")
        print(f"\n{Colors.BOLD}Services Breakdown:{Colors.RESET}")
        for s, count in sorted(services.items(), key=lambda x: x[1], reverse=True)[:15]:
            print(f"  {count:5d}x  {s}")

    # Loadout History & Reversions
    if args.loadouts or (not should_dump_frames and not args.summary and not args.timeline):
        print(f"\n{Colors.BOLD}{Colors.MAGENTA}--- LOADOUT & SPAWN REVERSION TRACKER ---{Colors.RESET}")
        last_body = None
        for item in loadout_history:
            ldata = item["loadout"]
            body_id = None
            if isinstance(ldata, list) and len(ldata) > 0 and isinstance(ldata[0], int):
                body_id = ldata[0]
            elif isinstance(ldata, dict) and "Products" in ldata and isinstance(ldata["Products"], list):
                if len(ldata["Products"]) > 0:
                    first = ldata["Products"][0]
                    body_id = first.get("ProductID") if isinstance(first, dict) else first

            body_str = format_product_id(body_id) if body_id is not None else "Unknown"
            reversion_flag = ""
            if last_body is not None and body_id is not None and body_id != last_body:
                reversion_flag = f" {Colors.RED}{Colors.BOLD}<< CHANGED/REVERTED from {format_product_id(last_body)}! >>{Colors.RESET}"
            if body_id is not None:
                last_body = body_id

            color = Colors.GREEN if "c2u" in item["kind"] or "req" in item["kind"] else Colors.BLUE
            print(f"Seq {item['seq']:3d} [{item['ts'][11:19]}] {color}{item['kind']:10s}{Colors.RESET} {item['service'][:26]:26s} Body: {Colors.YELLOW}{body_str:20s}{Colors.RESET}{reversion_flag}")
            print(f"         Full: {format_loadout_display(ldata)}")

    # Timeline
    if args.timeline:
        print(f"\n{Colors.BOLD}{Colors.CYAN}--- MATCH LIFECYCLE TIMELINE ---{Colors.RESET}")
        for ev in events:
            color = Colors.GREEN if "c2u" in ev["kind"] or "req" in ev["kind"] else Colors.YELLOW
            print(f"[{ev['ts']}] Seq {ev['seq']:3d} {color}{ev['label']}{Colors.RESET}")
            for l in ev["loadouts"]:
                print(f"    Loadout: {format_loadout_display(l[2])}")

    if not should_dump_frames and not args.loadouts and not args.timeline:
        print(f"\n{Colors.BOLD}{Colors.GREEN}Tip:{Colors.RESET} To dump ALL traffic frames with all client headers and payloads without truncation:")
        print(f"  {Colors.YELLOW}python traffic_inspector.py --all{Colors.RESET}")
        print(f"  {Colors.YELLOW}python traffic_inspector.py --client-only --all{Colors.RESET}  (all client requests & headers)")
        print(f"  {Colors.YELLOW}python traffic_inspector.py --filter \"<keyword>\"{Colors.RESET}   (search headers/body with full dump)")
        print(f"  {Colors.YELLOW}python traffic_inspector.py --seq 97{Colors.RESET}               (inspect specific frame)")

def tail_file(file_path, args):
    print(f"{Colors.GREEN}Tailing real-time traffic on {file_path}... (Press Ctrl+C to stop){Colors.RESET}\n")
    with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
        f.seek(0, 2)
        while True:
            line = f.readline()
            if not line:
                time.sleep(0.1)
                continue
            line = line.strip()
            if not line:
                continue
            try:
                frame = json.loads(line)
                print_frame(frame, show_body=not args.headers_only, show_headers=True)
            except Exception:
                pass

# Standalone Proxy Recorder to intercept and dump traffic from any application
class ProxyRecorderHandler(BaseHTTPRequestHandler):
    log_file_path = "rpc-probe.jsonl"
    seq_counter = 1000

    def do_CONNECT(self):
        # Tunnel CONNECT for HTTPS
        self.send_response(200, "Connection Established")
        self.end_headers()
        print(f"{Colors.YELLOW}[CONNECT]{Colors.RESET} {self.path}")

    def do_GET(self):
        self.handle_any_request("GET")

    def do_POST(self):
        self.handle_any_request("POST")

    def do_PUT(self):
        self.handle_any_request("PUT")

    def do_DELETE(self):
        self.handle_any_request("DELETE")

    def handle_any_request(self, method):
        content_len = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_len).decode("utf-8", errors="replace") if content_len > 0 else ""

        headers_list = [f"{k}: {v}" for k, v in self.headers.items()]
        seq = ProxyRecorderHandler.seq_counter
        ProxyRecorderHandler.seq_counter += 1

        frame = {
            "ts": datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ"),
            "seq": seq,
            "kind": "client-req",
            "host": self.headers.get("Host", self.path),
            "method": method,
            "path": self.path,
            "service": self.headers.get("PsyService", ""),
            "headers": headers_list,
            "body": body,
            "body_len": len(body),
        }

        # Print immediately to terminal
        print_frame(frame, show_body=True, show_headers=True)

        # Append to log file
        try:
            with open(ProxyRecorderHandler.log_file_path, "a", encoding="utf-8") as out:
                out.write(json.dumps(frame) + "\n")
        except Exception as e:
            print(f"Error logging frame: {e}")

        # Respond to client with 200 OK
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok","captured_by":"VelocityRL_Inspector"}\n')

def start_proxy_listener(port, log_file):
    ProxyRecorderHandler.log_file_path = log_file or "rpc-probe.jsonl"
    server = HTTPServer(("0.0.0.0", port), ProxyRecorderHandler)
    print(f"{Colors.BOLD}{Colors.GREEN}=== VelocityRL Live Traffic Recorder ==={Colors.RESET}")
    print(f"Listening on port {port} for traffic from ALL applications.")
    print(f"Point HTTP_PROXY or applications to: {Colors.YELLOW}http://127.0.0.1:{port}{Colors.RESET}")
    print(f"Logging captured frames to: {Colors.CYAN}{ProxyRecorderHandler.log_file_path}{Colors.RESET}")
    print("Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopping traffic recorder.")
        server.server_close()

def main():
    parser = argparse.ArgumentParser(description="VelocityRL Traffic & Client Header Inspector")
    parser.add_argument("path", nargs="?", default="", help="Path to rpc-probe.jsonl or diagnostics folder")
    parser.add_argument("-a", "--all", action="store_true", help="Dump ALL frames with full headers and payloads (no truncation)")
    parser.add_argument("-c", "--client-only", action="store_true", help="Dump only client outbound traffic (requests & c2u)")
    parser.add_argument("-s", "--server-only", action="store_true", help="Dump only server inbound traffic (responses & u2c)")
    parser.add_argument("--headers", action="store_true", help="Dump frames focusing on all client/server headers")
    parser.add_argument("--headers-only", action="store_true", help="Show headers without printing full body payload")
    parser.add_argument("--seq", type=str, default=None, help="Inspect specific sequence number or range (e.g. 97 or 90-105)")
    parser.add_argument("--filter", type=str, default="", help="Search packets by keyword across headers and body (untruncated)")
    parser.add_argument("--service", type=str, default="", help="Filter packets by PsyService name")
    parser.add_argument("--summary", action="store_true", help="Print overall traffic summary")
    parser.add_argument("--timeline", action="store_true", help="Show chronological match lifecycle timeline")
    parser.add_argument("--loadouts", action="store_true", help="Detailed loadout history and reversion alerts")
    parser.add_argument("-f", "--follow", action="store_true", help="Live follow/tail packets in real-time")
    parser.add_argument("--listen", type=int, nargs="?", const=8888, default=None, help="Start proxy server on PORT to record traffic from all apps")

    args = parser.parse_args()

    if args.listen is not None:
        start_proxy_listener(args.listen, args.path)
        return

    target_path = find_target_file(args.path)

    if not target_path:
        print(f"{Colors.RED}Error: Could not locate rpc-probe.jsonl.{Colors.RESET}")
        print("Please provide the path to rpc-probe.jsonl, run in the diagnostics directory, or use --listen to capture live traffic.")
        sys.exit(1)

    if args.follow:
        tail_file(target_path, args)
    else:
        run_inspector(target_path, args)

if __name__ == "__main__":
    main()
