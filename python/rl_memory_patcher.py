
import sys
import time

try:
    import pymem
except ImportError:
    print("Error: pymem is required. Please run 'pip install pymem'")
    sys.exit(1)

def scan_and_replace(pm: pymem.Pymem, target: bytes, replacement: bytes):
    # Ensure replacement fits in the same or smaller buffer
    if len(replacement) > len(target):
        print(f"[!] Error: Replacement string '{replacement.decode()}' ({len(replacement)} bytes) is longer than target '{target.decode()}' ({len(target)} bytes).")
        print("    In-place memory replacement cannot safely expand strings without corrupting memory pools.")
        return False

    # Pad replacement with null bytes so we overwrite exactly the same length
    padded_replacement = replacement.ljust(len(target), b'\x00')
    
    # We search for the exact target string followed by a null terminator
    search_bytes = target + b'\x00'
    padded_replacement_with_null = padded_replacement + b'\x00'

    print(f"[*] Scanning memory for: {target.decode()}...")
    
    # Track how many replacements we make
    count = 0
    
    # We need to scan the process memory regions.
    # Pymem provides pattern scanning, but iterating memory pages manually for raw bytes is often faster.
    
    system_info = pymem.process.get_system_info()
    min_address = system_info.lpMinimumApplicationAddress
    max_address = system_info.lpMaximumApplicationAddress

    # Iterate through memory regions
    address = min_address
    
    # Note: Memory scanning large processes (Rocket League is ~2GB+) in Python can be slow.
    # We optimize by only checking MEM_COMMIT pages that are readable and writable.
    
    while address < max_address:
        mbi = pymem.memory.virtual_query(pm.process_handle, address)
        
        # We only care about committed, readable/writable memory (no execute, no guard pages)
        # Usually strings in UE3 FName pools are in PAGE_READWRITE
        if mbi.State == 0x1000 and (mbi.Protect == 0x04 or mbi.Protect == 0x40): # MEM_COMMIT, PAGE_READWRITE / PAGE_EXECUTE_READWRITE
            try:
                buffer = pm.read_bytes(mbi.BaseAddress, mbi.RegionSize)
                
                # Find all occurrences in this chunk
                offset = 0
                while True:
                    offset = buffer.find(search_bytes, offset)
                    if offset == -1:
                        break
                        
                    # Found it! Calculate absolute memory address
                    match_address = mbi.BaseAddress + offset
                    
                    # Write the new padded string over the old one
                    pm.write_bytes(match_address, padded_replacement_with_null, len(padded_replacement_with_null))
                    
                    print(f"[+] Replaced occurrence at 0x{match_address:X}")
                    count += 1
                    offset += len(search_bytes)
                    
            except pymem.exception.MemoryReadError:
                # Some pages might fail to read if they have weird permissions, just skip
                pass
                
        address += mbi.RegionSize

    print(f"[*] Done. Replaced {count} instances.")
    return count > 0

def main():
    if len(sys.argv) != 3:
        print("Usage: python rl_memory_patcher.py <OriginalItem> <ReplacementItem>")
        print("Example: python rl_memory_patcher.py Flamethrower AlphaReward")
        sys.exit(1)

    original_item = sys.argv[1].encode('ascii')
    replacement_item = sys.argv[2].encode('ascii')

    print("[*] Connecting to RocketLeague.exe...")
    try:
        pm = pymem.Pymem("RocketLeague.exe")
    except pymem.exception.ProcessNotFound:
        print("[!] RocketLeague.exe is not currently running. Please launch the game first.")
        sys.exit(1)
        
    print(f"[+] Connected to process (PID: {pm.process_id})")
    
    start_time = time.time()
    scan_and_replace(pm, original_item, replacement_item)
    elapsed = time.time() - start_time
    
    print(f"[*] Memory patch operation completed in {elapsed:.2f} seconds.")

if __name__ == "__main__":
    main()
