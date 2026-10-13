#!/usr/bin/env python3
"""
VelocityRL Traffic Inspector & Loadout Reversion Analyzer
Author: VelocityRL Team
Usage:
    python traffic_inspector.py [PATH_TO_RPC_PROBE_OR_DIR] [OPTIONS]

Options:
    --summary           Print summary of all services, sessions, and loadouts
    --timeline          Show chronological game lifecycle (reservations, goals, loadouts)
    --loadouts          Track all loadouts sent or received, highlighting reversions
    --filter KEYWORD    Filter frames containing KEYWORD (case-insensitive)
    --service SERVICE   Filter specific PsyService (e.g., 'DSR/RelayToServer')
    --follow, -f        Live tail the log file as new packets arrive
"""

import sys
import os
import json
import time
import argparse
from datetime import datetime

# ANSI Color codes for readable terminal display
class Colors:
    RESET = "\033[0m"
    BOLD = "\033[1m"
    RED = "\033[91m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    BLUE = "\033[94m"
    MAGENTA = "\033[95m"
    CYAN = "\033[96m"
    GRAY = "\033[90m"

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
        return path_arg
    
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
    ])

    for c in candidates:
        if os.path.isfile(c):
            return os.path.abspath(c)
    return None

def parse_inner_payload(body_raw):
    """Attempt to parse outer and nested stringified JSON payloads."""
    if not body_raw:
        return None, None
    try:
        outer = json.loads(body_raw)
    except Exception:
        return None, None

    inner = None
    if isinstance(outer, dict):
        # Rocket League often nests JSON strings in MessagePayload or EventData
        for key in ["MessagePayload", "EventData", "Data"]:
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
        # Positional integer array [Body, (Decal), Wheels, Boost, Trail, GoalExplosion]
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

def analyze_traffic(file_path, args):
    print(f"{Colors.BOLD}{Colors.CYAN}=== VelocityRL Traffic Inspector ==={Colors.RESET}")
    print(f"Reading: {Colors.YELLOW}{file_path}{Colors.RESET}\n")

    services = {}
    servers = set()
    loadout_history = []
    events = []
    total_frames = 0
    c2u_count = 0
    u2c_count = 0

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

            if "c2u" in kind:
                c2u_count += 1
            elif "u2c" in kind:
                u2c_count += 1

            services[svc] = services.get(svc, 0) + 1

            outer, inner = parse_inner_payload(body_raw)

            # Check for server address
            if outer and isinstance(outer, dict):
                addr = outer.get("ServerAddress")
                if addr:
                    servers.add(addr)
            if inner and isinstance(inner, dict):
                addr = inner.get("ServerAddress")
                if addr:
                    servers.add(addr)

            # Extract loadout occurrences
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

            # Matchmaking & Reservation lifecycle events
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

            # Check keyword filter if requested
            if args.filter:
                kw = args.filter.lower()
                if kw in line.lower():
                    print(f"{Colors.GRAY}[Line {line_num} | Seq {seq} | {kind}]{Colors.RESET} {Colors.CYAN}{svc}{Colors.RESET}")
                    # Print relevant excerpt
                    for entry in found_loadouts:
                        print(f"  -> {Colors.GREEN}Loadout:{Colors.RESET}{format_loadout_display(entry[2])}")
                    if len(line) < 300:
                        print(f"  {line}")
                    else:
                        print(f"  {line[:300]}... [truncated]")

            if args.service and args.service.lower() in svc.lower():
                print(f"{Colors.YELLOW}[Seq {seq} | {ts} | {kind}]{Colors.RESET} {Colors.CYAN}{svc}{Colors.RESET}")
                if found_loadouts:
                    for entry in found_loadouts:
                        print(f"  -> {Colors.GREEN}{entry[1]} ({entry[0]}):{Colors.RESET}{format_loadout_display(entry[2])}")

    # Mode: Summary
    if args.summary or (not args.timeline and not args.loadouts and not args.filter and not args.service):
        print(f"{Colors.BOLD}--- TRAFFIC SUMMARY ---{Colors.RESET}")
        print(f"Total Frames: {total_frames} (Client->Server: {c2u_count}, Server->Client: {u2c_count})")
        print(f"Dedicated Servers Seen: {len(servers)}")
        for s in list(servers)[:5]:
            print(f"  * {s}")
        print(f"\n{Colors.BOLD}Services Breakdown:{Colors.RESET}")
        for s, count in sorted(services.items(), key=lambda x: x[1], reverse=True)[:15]:
            print(f"  {count:5d}x  {s}")

    # Mode: Loadout Reversion Analysis
    if args.loadouts or not (args.timeline or args.filter or args.service):
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

            color = Colors.GREEN if "c2u" in item["kind"] else Colors.BLUE
            print(f"Seq {item['seq']:3d} [{item['ts'][11:19]}] {color}{item['kind']:6s}{Colors.RESET} {item['service'][:26]:26s} Body: {Colors.YELLOW}{body_str:20s}{Colors.RESET}{reversion_flag}")
            print(f"         Full: {format_loadout_display(ldata)}")

    # Mode: Timeline
    if args.timeline:
        print(f"\n{Colors.BOLD}{Colors.CYAN}--- MATCH LIFECYCLE TIMELINE ---{Colors.RESET}")
        for ev in events:
            color = Colors.GREEN if "c2u" in ev["kind"] else Colors.YELLOW
            print(f"[{ev['ts']}] Seq {ev['seq']:3d} {color}{ev['label']}{Colors.RESET}")
            for l in ev["loadouts"]:
                print(f"    Loadout: {format_loadout_display(l[2])}")

def tail_file(file_path):
    print(f"{Colors.GREEN}Listening to real-time traffic on {file_path}... (Press Ctrl+C to stop){Colors.RESET}\n")
    with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
        f.seek(0, 2) # end of file
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
                kind = frame.get("kind", "")
                svc = frame.get("service", "")
                seq = frame.get("seq", 0)
                color = Colors.GREEN if "c2u" in kind else Colors.BLUE
                print(f"[{datetime.now().strftime('%H:%M:%S')}] Seq {seq:3d} {color}{kind:6s}{Colors.RESET} {svc}")
                outer, inner = parse_inner_payload(frame.get("body", ""))
                loads = extract_loadouts(outer, inner)
                for item in loads:
                    print(f"  {Colors.YELLOW}-> Loadout:{Colors.RESET} {format_loadout_display(item[2])}")
            except Exception:
                pass

def main():
    parser = argparse.ArgumentParser(description="VelocityRL Traffic & Reversion Inspector")
    parser.add_argument("path", nargs="?", default="", help="Path to rpc-probe.jsonl or diagnostics folder")
    parser.add_argument("--summary", action="store_true", help="Print overall traffic summary")
    parser.add_argument("--timeline", action="store_true", help="Show chronological match lifecycle timeline")
    parser.add_argument("--loadouts", action="store_true", help="Detailed loadout history and reversion alerts")
    parser.add_argument("--filter", type=str, default="", help="Search packets by keyword")
    parser.add_argument("--service", type=str, default="", help="Filter packets by PsyService name")
    parser.add_argument("-f", "--follow", action="store_true", help="Live follow/tail packets in real-time")

    args = parser.parse_args()
    target_path = find_target_file(args.path)

    if not target_path:
        print(f"{Colors.RED}Error: Could not locate rpc-probe.jsonl.{Colors.RESET}")
        print("Please provide the path to rpc-probe.jsonl or run in the diagnostics directory.")
        sys.exit(1)

    if args.follow:
        tail_file(target_path)
    else:
        analyze_traffic(target_path, args)

if __name__ == "__main__":
    main()
