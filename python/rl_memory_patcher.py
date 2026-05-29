
import sys
import time
import argparse

try:
    import pymem
    import psutil
except ImportError:
    print("Error: pymem and psutil are required. Please run 'pip install pymem psutil'")
    sys.exit(1)

def scan_and_replace(pm, target, replacement):
    # Pad replacement with null bytes so we overwrite exactly the same length
    if len(replacement) > len(target):
        print(f"[!] Error: Replacement '{replacement.decode()}' is longer than target '{target.decode()}'.")
        return False

    padded_replacement = replacement.ljust(len(target), b'\x00')
    search_bytes = target + b'\x00'
    padded_replacement_with_null = padded_replacement + b'\x00'

    print(f"[*] Scanning memory for: {target.decode()}...")
    count = 0
    
    # Efficient scan using virtual_query
    try:
        system_info = pymem.process.get_system_info()
        min_address = system_info.lpMinimumApplicationAddress
        max_address = system_info.lpMaximumApplicationAddress
        address = min_address
        
        while address < max_address:
            try:
                mbi = pymem.memory.virtual_query(pm.process_handle, address)
                # MEM_COMMIT = 0x1000, PAGE_READWRITE = 0x04, PAGE_EXECUTE_READWRITE = 0x40
                if mbi.State == 0x1000 and (mbi.Protect & 0x04 or mbi.Protect & 0x40):
                    buffer = pm.read_bytes(mbi.BaseAddress, mbi.RegionSize)
                    offset = 0
                    while True:
                        offset = buffer.find(search_bytes, offset)
                        if offset == -1:
                            break
                        match_address = mbi.BaseAddress + offset
                        pm.write_bytes(match_address, padded_replacement_with_null, len(padded_replacement_with_null))
                        print(f"[+] Replaced occurrence at 0x{match_address:X}")
                        count += 1
                        offset += len(search_bytes)
                address += mbi.RegionSize
            except (pymem.exception.MemoryReadError, pymem.exception.WinAPIError):
                # Move to next page if we hit an error
                address += 4096 # Standard page size fallback
    except Exception as e:
        print(f"[!] Error during scan: {e}")

    print(f"[*] Done. Replaced {count} instances.")
    return count > 0

def main():
    parser = argparse.ArgumentParser(description="Rocket League Memory Patcher")
    parser.add_argument("--old", required=True, help="Current display name")
    parser.add_argument("--new", required=True, help="New display name")
    args = parser.parse_args()

    target = args.old.encode('utf-8')
    replacement = args.new.encode('utf-8')

    print("[*] Connecting to RocketLeague.exe...")
    try:
        pm = pymem.Pymem("RocketLeague.exe")
    except pymem.exception.ProcessNotFound:
        print("[!] RocketLeague.exe is not running.")
        sys.exit(1)
        
    print(f"[+] Connected to PID: {pm.process_id}")
    start_time = time.time()
    scan_and_replace(pm, target, replacement)
    print(f"[*] Completed in {time.time() - start_time:.2f}s")

if __name__ == "__main__":
    main()
