import sys
import os

# Add RLUPKTools to path
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')

import rl_upk_editor
import base64

file_path = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk'
keys_path = 'c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools\\keys.txt'

provider = rl_upk_editor.DecryptionProvider(keys_path)

with open(file_path, "rb") as f:
    summary = rl_upk_editor.parse_file_summary(f)
    meta = rl_upk_editor.parse_file_compression_metadata(f)
    
    # Simulating what rl_upk_editor does
    encrypted_size = summary.total_header_size - meta.garbage_size - summary.name_offset
    encrypted_size = (encrypted_size + 15) & ~15
    f.seek(summary.name_offset)
    encrypted_data = f.read(encrypted_size)
    
    valid_key = None
    for key in provider.decryption_keys:
        if rl_upk_editor.verify_decryptor(summary, meta, key, encrypted_data):
            valid_key = key
            break
            
    if valid_key:
        print(f"Key (Base64): {base64.b64encode(valid_key).decode()}")
    else:
        print("No key found via verify_decryptor")
