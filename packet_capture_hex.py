#!/usr/bin/env python3
"""
VelocityRL Raw Packet Hex Capture Tool
Captures ALL network traffic (UDP, TCP, ICMP) across ALL ports on Windows or Linux
and saves/prints raw bytes in clean hexadecimal so no binary data is corrupted by text encoding.

Usage:
    python packet_capture_hex.py [OPTIONS]

Options:
    --output FILE, -o FILE    Save captured packets to file (default: raw_traffic_dump.jsonl)
    --udp-only                Capture only UDP packets (gameplay replication, car respawn)
    --tcp-only                Capture only TCP packets (PsyNet RPC, WebSockets)
    --port PORT               Filter specific port (e.g. 7000-9000 for RL match servers)
    --server-ip IP            Filter by specific server IP
    --max-packets N           Stop automatically after N packets
"""

import sys
import os
import socket
import struct
import json
import time
from datetime import datetime

def get_default_local_ip():
    """Find the default active local IP address."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        # Does not actually connect or send data
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
    except Exception:
        ip = "127.0.0.1"
    finally:
        s.close()
    return ip

def format_hexdump(data, length=16):
    """Format bytes as a classic side-by-side hex and ASCII view."""
    lines = []
    for i in range(0, len(data), length):
        chunk = data[i:i + length]
        hex_bytes = " ".join(f"{b:02x}" for b in chunk)
        ascii_text = "".join(chr(b) if 32 <= b <= 126 else "." for b in chunk)
        lines.append(f"{i:04x}   {hex_bytes:<{length*3}}  |{ascii_text}|")
    return "\n".join(lines)

def main():
    import argparse
    parser = argparse.ArgumentParser(description="Raw Packet Hex Capture")
    parser.add_argument("-o", "--output", default="raw_traffic_dump.jsonl", help="Output file path")
    parser.add_argument("--udp-only", action="store_true", help="Capture only UDP packets")
    parser.add_argument("--tcp-only", action="store_true", help="Capture only TCP packets")
    parser.add_argument("--port", type=str, default=None, help="Filter port or range (e.g. 9056 or 7000-9500)")
    parser.add_argument("--server-ip", type=str, default=None, help="Filter by server IP")
    parser.add_argument("--max-packets", type=int, default=0, help="Stop after N packets (0 = unlimited)")
    parser.add_argument("--no-file", action="store_true", help="Don't write to file, console only")
    args = parser.parse_args()

    port_min, port_max = None, None
    if args.port:
        if "-" in args.port:
            parts = args.port.split("-", 1)
            port_min, port_max = int(parts[0]), int(parts[1])
        else:
            port_min = port_max = int(args.port)

    is_windows = os.name == "nt" or sys.platform.startswith("win")
    local_ip = get_default_local_ip()

    print("=" * 70)
    print(" VelocityRL Raw Network Hex Capture")
    print(f" Platform: {'Windows' if is_windows else 'Linux/POSIX'} | Local IP: {local_ip}")
    print(f" Output: {args.output if not args.no_file else '(console only)'}")
    print("=" * 70)

    # Create raw socket
    try:
        if is_windows:
            sniffer = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_IP)
            sniffer.bind((local_ip, 0))
            sniffer.ioctl(socket.SIO_RCVALL, socket.RCVALL_ON)
        else:
            sniffer = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_UDP if args.udp_only else socket.IPPROTO_TCP)
            sniffer.bind(("0.0.0.0", 0))
    except PermissionError:
        print("\n[ERROR] Raw socket requires Administrator / Root privileges!")
        if is_windows:
            print("Please right-click Command Prompt / PowerShell and select 'Run as Administrator'.")
        else:
            print("Please run with 'sudo python packet_capture_hex.py'.")
        sys.exit(1)
    except Exception as e:
        print(f"\n[ERROR] Failed to initialize raw socket on {local_ip}: {e}")
        sys.exit(1)

    print("\nCapturing live traffic... Press Ctrl+C to stop.\n")

    out_file = None
    if not args.no_file:
        out_file = open(args.output, "a", encoding="utf-8")

    packet_count = 0

    try:
        while True:
            raw_data, _ = sniffer.recvfrom(65535)
            if len(raw_data) < 20:
                continue

            # Parse IPv4 header
            ip_header = raw_data[0:20]
            iph = struct.unpack('!BBHHHBBH4s4s', ip_header)
            version_ihl = iph[0]
            ihl = (version_ihl & 0xF) * 4
            protocol = iph[6]
            src_ip = socket.inet_ntoa(iph[8])
            dst_ip = socket.inet_ntoa(iph[9])

            proto_name = "OTHER"
            src_port = 0
            dst_port = 0
            payload = b""

            if protocol == 17: # UDP
                if args.tcp_only:
                    continue
                proto_name = "UDP"
                if len(raw_data) >= ihl + 8:
                    udp_header = raw_data[ihl:ihl + 8]
                    udph = struct.unpack('!HHHH', udp_header)
                    src_port = udph[0]
                    dst_port = udph[1]
                    payload = raw_data[ihl + 8:]
            elif protocol == 6: # TCP
                if args.udp_only:
                    continue
                proto_name = "TCP"
                if len(raw_data) >= ihl + 20:
                    tcp_header = raw_data[ihl:ihl + 20]
                    tcph = struct.unpack('!HHLLBBHHH', tcp_header)
                    src_port = tcph[0]
                    dst_port = tcph[1]
                    tcp_data_offset = (tcph[4] >> 4) * 4
                    payload = raw_data[ihl + tcp_data_offset:]
            else:
                continue

            # Filter IP if requested
            if args.server_ip:
                if src_ip != args.server_ip and dst_ip != args.server_ip:
                    continue

            # Filter Port if requested
            if port_min is not None:
                if not ((port_min <= src_port <= port_max) or (port_min <= dst_port <= port_max)):
                    continue

            if not payload:
                continue

            packet_count += 1
            ts = datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%S.%fZ")
            hex_payload = payload.hex()

            record = {
                "packet_id": packet_count,
                "ts": ts,
                "proto": proto_name,
                "src": f"{src_ip}:{src_port}",
                "dst": f"{dst_ip}:{dst_port}",
                "len": len(payload),
                "hex": hex_payload,
            }

            # Console preview
            direction = f"{src_ip}:{src_port} -> {dst_ip}:{dst_port}"
            preview_hex = hex_payload[:48] + ("..." if len(hex_payload) > 48 else "")
            print(f"[#{packet_count:04d} | {proto_name}] {direction} ({len(payload)}B): {preview_hex}")

            # Write clean JSON record with exact raw hex
            if out_file:
                out_file.write(json.dumps(record) + "\n")
                out_file.flush()

            if args.max_packets and packet_count >= args.max_packets:
                print(f"\nReached max packets ({args.max_packets}). Stopping.")
                break

    except KeyboardInterrupt:
        print(f"\nCaptured {packet_count} packets. Stopped by user.")
    finally:
        if is_windows:
            try:
                sniffer.ioctl(socket.SIO_RCVALL, socket.RCVALL_OFF)
            except Exception:
                pass
        sniffer.close()
        if out_file:
            out_file.close()
            print(f"All packets safely written in raw hex to: {os.path.abspath(args.output)}")

if __name__ == "__main__":
    main()
