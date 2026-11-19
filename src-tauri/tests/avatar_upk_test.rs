// Integration test that verifies the avatar UPK patch produces a valid header
// whose repointed GFxData_PRI_TA::UpdatePRIData export contains the transplanted
// sidecar-avatar bytecode. Skips when no stock UPK fixture is available.
use std::fs;
use std::path::PathBuf;

#[test]
fn transplanted_update_pri_data_is_readable() {
    let fixture = std::env::var("TAGAME_TEST_UPK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/root/velrlapi/.cache/TAGame.upk.bak"));
    if !fixture.is_file() {
        eprintln!("skipping: no UPK fixture at {}", fixture.display());
        return;
    }

    let tmp = std::env::temp_dir().join(format!(
        "velocityrl_avatar_patch_test_{}.upk",
        std::process::id()
    ));
    fs::copy(&fixture, &tmp).unwrap();

    let raw_bgra = vec![0x80u8; 262_144];
    app_lib::avatar::apply_custom_avatar_to_tagame(&tmp, &raw_bgra)
        .expect("patch should apply");

    let bytes = fs::read(&tmp).unwrap();
    assert!(bytes.len() > 32);

    let total_header_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4;
    let name_count = i32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let name_offset = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
    let enc_size = (total_header_size - name_offset + 15) & !15;
    let enc_end = name_offset + enc_size;
    let plain = app_lib::upk::crypto::decrypt_ecb(
        &app_lib::upk::swapper::TAGAME_KEY,
        &bytes[name_offset..enc_end],
    );
    let names = app_lib::upk::palette::parse_names_in_block(&plain, name_count).unwrap();

    let p_sum = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize }) + 4 + 8;
    let export_count = i32::from_le_bytes(bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
    let export_offset = i32::from_le_bytes(bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
    let depends_offset = i32::from_le_bytes(bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;
    let export_rel = export_offset - name_offset;
    let depends_rel = depends_offset - name_offset;

    let mut pos = export_rel;
    let mut found = false;
    for _ in 0..export_count {
        if pos + 72 > depends_rel || pos + 72 > plain.len() {
            break;
        }
        let i32_at = |a: usize| i32::from_le_bytes(plain[pos + a..pos + a + 4].try_into().unwrap());
        let name_idx = i32_at(12);
        let serial_size = i32_at(32);
        let serial_offset = i64::from_le_bytes(plain[pos + 36..pos + 44].try_into().unwrap());
        let name = if name_idx >= 0 && (name_idx as usize) < names.len() {
            names[name_idx as usize].clone()
        } else {
            String::new()
        };
        if name == "UpdatePRIData" && serial_size > 19_000 {
            // Locate the owning chunk and verify the transplanted script bytes.
            let chunk_table_pos = depends_rel;
            let chunk_count =
                i32::from_le_bytes(plain[chunk_table_pos..chunk_table_pos + 4].try_into().unwrap())
                    as usize;
            let mut cp = chunk_table_pos + 4;
            for _ in 0..chunk_count {
                let uncomp_offset = i64::from_le_bytes(plain[cp..cp + 8].try_into().unwrap());
                let uncomp_size = i32::from_le_bytes(plain[cp + 8..cp + 12].try_into().unwrap());
                let comp_offset = i64::from_le_bytes(plain[cp + 12..cp + 20].try_into().unwrap());
                let comp_size = i32::from_le_bytes(plain[cp + 20..cp + 24].try_into().unwrap()) as usize;
                if serial_offset >= uncomp_offset
                    && serial_offset < uncomp_offset + uncomp_size as i64
                {
                    let start = comp_offset as usize;
                    let decomp = app_lib::upk::compression::decompress_chunk(
                        &bytes[start..start + comp_size],
                    )
                    .expect("chunk should decompress");
                    let rel = (serial_offset - uncomp_offset) as usize;
                    let script = &decomp[rel + 48..rel + 48 + 19_434];
                    let expected: &[u8] =
                        include_bytes!("../src/avatar/shift_update_pri_data.bin");
                    assert_eq!(script, expected, "transplanted bytecode mismatch");
                    found = true;
                    break;
                }
                cp += 36;
            }
            break;
        }
        let noc = i32_at(48);
        pos += 72 + noc.max(0) as usize * 4;
    }
    assert!(found, "transplanted UpdatePRIData export not found");

    // No two exports may share serial data. Appending the transplanted block
    // grows the chunk, so every later export and chunk must be shifted; without
    // that, the game crashes with "duplicate NetIndex / Serial size mismatch".
    let mut serials: Vec<(i32, i64, String, String)> = Vec::new();
    let mut pos = export_rel;
    for _ in 0..export_count {
        if pos + 72 > depends_rel || pos + 72 > plain.len() {
            break;
        }
        let i32_at = |a: usize| i32::from_le_bytes(plain[pos + a..pos + a + 4].try_into().unwrap());
        let name_idx = i32_at(12);
        let serial_size = i32_at(32);
        let serial_offset = i64::from_le_bytes(plain[pos + 36..pos + 44].try_into().unwrap());
        let name = if name_idx >= 0 && (name_idx as usize) < names.len() {
            names[name_idx as usize].clone()
        } else {
            String::new()
        };
        if serial_size > 0 {
            serials.push((serial_size, serial_offset, name, String::new()));
        }
        let noc = i32_at(48);
        pos += 72 + noc.max(0) as usize * 4;
    }
    serials.sort_by_key(|s| s.1);
    for pair in serials.windows(2) {
        assert!(
            pair[1].1 >= pair[0].1 + pair[0].0 as i64,
            "exports overlap: {} @ {} (size {}) and {} @ {}",
            pair[0].2, pair[0].1, pair[0].0, pair[1].2, pair[1].1
        );
    }
    let force_feedback = serials
        .iter()
        .find(|s| s.2 == "SetForceFeedbackScale")
        .expect("SetForceFeedbackScale must still be present");
    assert_eq!(force_feedback.0, 96, "neighbouring export serial size changed");

    let _ = fs::remove_file(&tmp);
}
