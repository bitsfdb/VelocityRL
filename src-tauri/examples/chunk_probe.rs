use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args
        .first()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"E:\games\rocketleague\TAGame\CookedPCConsole"));
    let keys = include_str!("../resources/keys.txt");
    let keymap = include_str!("../resources/keys_map.json");
    let tagame = dir.join("TAGame.upk");
    let data = std::fs::read(&tagame).expect("read TAGame");
    let (summary, meta, plain, _, _) = app_lib::upk::palette::debug_decrypt(&data, keys, keymap).expect("decrypt");
    let chunks = app_lib::upk::parser::parse_chunks(&plain, meta.compressed_chunks_offset).expect("chunks");
    println!("file len = {}", data.len());
    println!("chunk stride/table @ {}", meta.compressed_chunks_offset);
    for (i, c) in chunks.iter().enumerate() {
        println!(
            "chunk {i}: uoff={} usize={} coff={} csize={} (coff+csize={})",
            c.uncompressed_offset,
            c.uncompressed_size,
            c.compressed_offset,
            c.compressed_size,
            c.compressed_offset + c.compressed_size as i64
        );
    }
    let _ = summary;
    let _ = Path::new("");
}
