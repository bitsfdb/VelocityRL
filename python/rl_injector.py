
import ctypes
import os
import sys

def inject_dll(process_name, dll_path):
    """
    Injects a DLL into a running process by name using standard Win32 API calls.
    """
    try:
        import psutil
    except ImportError:
        print("Error: psutil is required. Please run: pip install psutil")
        sys.exit(1)

    print(f"[*] Looking for process: {process_name}")
    pid = None
    for proc in psutil.process_iter(['pid', 'name']):
        if proc.info['name'].lower() == process_name.lower():
            pid = proc.info['pid']
            break

    if not pid:
        print(f"[!] Process {process_name} not found.")
        sys.exit(1)

    print(f"[*] Found {process_name} running at PID: {pid}")

    if not os.path.exists(dll_path):
        print(f"[!] DLL not found at: {dll_path}")
        sys.exit(1)

    dll_path_bytes = dll_path.encode('utf-8')
    dll_len = len(dll_path_bytes) + 1

    # Win32 API Definitions
    kernel32 = ctypes.windll.kernel32
    
    PROCESS_ALL_ACCESS = (0x000F0000 | 0x00100000 | 0xFFF)
    MEM_COMMIT = 0x1000
    MEM_RESERVE = 0x2000
    PAGE_READWRITE = 0x04

    # 1. Open the target process
    print("[*] Opening target process...")
    h_process = kernel32.OpenProcess(PROCESS_ALL_ACCESS, False, pid)
    if not h_process:
        print(f"[!] Failed to acquire handle to PID {pid}")
        sys.exit(1)

    # 2. Allocate memory in the target process for the DLL path
    print("[*] Allocating memory in target process...")
    arg_address = kernel32.VirtualAllocEx(h_process, 0, dll_len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)
    if not arg_address:
        print("[!] Failed to allocate memory.")
        kernel32.CloseHandle(h_process)
        sys.exit(1)

    # 3. Write the DLL path into the allocated memory
    print("[*] Writing DLL path into target memory...")
    written = ctypes.c_int(0)
    kernel32.WriteProcessMemory(h_process, arg_address, dll_path_bytes, dll_len, ctypes.byref(written))

    # 4. Get the address of LoadLibraryA in kernel32.dll
    print("[*] Resolving LoadLibraryA...")
    h_kernel32 = kernel32.GetModuleHandleA(b"kernel32.dll")
    h_loadlib = kernel32.GetProcAddress(h_kernel32, b"LoadLibraryA")

    # 5. Create a remote thread that calls LoadLibraryA with our DLL path
    print("[*] Spawning remote thread to execute payload...")
    thread_id = ctypes.c_ulong(0)
    h_thread = kernel32.CreateRemoteThread(h_process, None, 0, h_loadlib, arg_address, 0, ctypes.byref(thread_id))
    
    if not h_thread:
        print("[!] Failed to create remote thread.")
        kernel32.VirtualFreeEx(h_process, arg_address, 0, 0x8000) # MEM_RELEASE
        kernel32.CloseHandle(h_process)
        sys.exit(1)

    print(f"[+] Injection successful! Thread ID: {thread_id.value}")

    # Clean up handles
    kernel32.CloseHandle(h_thread)
    kernel32.CloseHandle(h_process)

def generate_config(target_item, replacement_item):
    config_path = os.path.join(os.path.dirname(__file__), "..", "payload_config.txt")
    with open(config_path, "w") as f:
        f.write(f"{target_item}\n")
        f.write(f"{replacement_item}\n")
    print(f"[*] Config written to {config_path}")

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="RLItemMod Injector")
    parser.add_argument("--target", required=True, help="Original item name (e.g., Flamethrower)")
    parser.add_argument("--replace", required=True, help="Replacement item name (e.g., AlphaReward)")
    parser.add_argument("--dll", default=os.path.join(os.path.dirname(__file__), "..", "src", "hook_payload", "Payload.dll"), help="Path to payload DLL")
    
    args = parser.parse_args()
    
    generate_config(args.target, args.replace)
    
    print("[*] Preparing to inject...")
    inject_dll("RocketLeague.exe", os.path.abspath(args.dll))

