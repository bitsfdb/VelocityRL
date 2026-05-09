import sys
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')
import rl_upk_editor

file_path = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk'

with open(file_path, "rb") as f:
    summary = rl_upk_editor.parse_file_summary(f)
    meta = rl_upk_editor.parse_file_compression_metadata(f)
    provider = rl_upk_editor.DecryptionProvider('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools\\keys.txt')
    f.seek(0)
    header_bytes = f.read(summary.name_offset)
    decrypted_data = rl_upk_editor.decrypt_data(f, summary, meta, provider)
    
    import io
    bio = io.BytesIO(header_bytes + decrypted_data)
    bio.seek(summary.export_offset)
    r = rl_upk_editor.BinaryReader(bio)
    
    # Read first export
    r.seek(summary.export_offset + 48) # Skip to NetObjCount (12 fields * 4 bytes?)
    # Actually let's just parse the full export properly.
    
    class_index = r.read_i32()
    super_index = r.read_i32()
    outer_index = r.read_i32()
    object_name_index = r.read_i32()
    object_name_number = r.read_i32()
    archetype_index = r.read_i32()
    object_flags = r.read_u64()
    serial_size = r.read_i32()
    serial_offset = r.read_i32()
    export_flags = r.read_i32()
    
    net_object_count = r.read_i32()
    net_objects = [r.read_i32() for _ in range(net_object_count)]
    
    # We can't access stream directly, so we just read 4 bytes 4 times for GUID
    guid1 = r.read_i32()
    guid2 = r.read_i32()
    guid3 = r.read_i32()
    guid4 = r.read_i32()
    
    package_flags = r.read_i32()
    
    print(f"NetObjects: {net_objects}")
    print(f"PackageFlags: {package_flags}")
    
    print("Export 1 class index:", r.read_i32())
