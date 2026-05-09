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
    
    for i in range(35):
        pos_before = bio.tell()
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
        print(f"Export {i}: NetObjCount = {net_object_count}, pos before skip: {pos_before + 44}")
        if net_object_count < 0 or net_object_count > 1000:
            print(f"Abnormal! Skipping {net_object_count} net objects...")
        for _ in range(net_object_count):
            r.read_i32()
        guid = bio.read(16)
        package_flags = r.read_i32()
        
