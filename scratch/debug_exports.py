import sys
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')
import rl_upk_editor

file_path = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk'

with open(file_path, "rb") as f:
    summary = rl_upk_editor.parse_file_summary(f)
    print(f"Export Count: {summary.export_count}")
    print(f"Export Offset: {summary.export_offset}")
    
    # Decrypt the header
    meta = rl_upk_editor.parse_file_compression_metadata(f)
    provider = rl_upk_editor.DecryptionProvider('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools\\keys.txt')
    f.seek(0)
    header_bytes = f.read(summary.name_offset)
    decrypted_data = rl_upk_editor.decrypt_data(f, summary, meta, provider)
    
    # Read first export
    import io
    bio = io.BytesIO(header_bytes + decrypted_data)
    bio.seek(summary.export_offset)
    r = rl_upk_editor.BinaryReader(bio)
    
    print(f"ClassIndex: {r.read_i32()}")
    print(f"SuperIndex: {r.read_i32()}")
    print(f"OuterIndex: {r.read_i32()}")
    print(f"ObjectNameIndex: {r.read_i32()}")
    print(f"ObjectNameNumber: {r.read_i32()}")
    print(f"ArchetypeIndex: {r.read_i32()}")
    print(f"ObjectFlags: {r.read_u64()}")
    print(f"SerialSize: {r.read_i32()}")
    print(f"SerialOffset: {r.read_i32()}")
    print(f"ExportFlags: {r.read_i32()}")
    
    net_obj_count = r.read_i32()
    print(f"NetObjCount: {net_obj_count}")
