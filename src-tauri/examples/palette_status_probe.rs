use app_lib::upk::palette;

fn main() {
    let game_dir = std::path::Path::new(r"E:\games\rocketleague");
    let cooked = palette::resolve_cooked_dir(game_dir).expect("cooked dir");
    let tagame = cooked.join("TAGame.upk");
    let data = std::fs::read(&tagame).expect("read tagame");
    let keys = include_str!("../resources/keys.txt");
    let keymap = include_str!("../resources/keys_map.json");
    let (summary, meta, plain, _, _) =
        palette::debug_decrypt(&data, keys, keymap).expect("decrypt");
    let sets = palette::debug_color_sets(&plain, &summary).expect("color sets");
    println!("sets found:");
    let mut names: Vec<_> = sets.keys().collect();
    names.sort();
    for n in names {
        let (count, size, off) = sets[n];
        println!("  {n}: elem_count={count} serial_size={size} offset={off}");
    }
    match palette::debug_classify(&data, &plain, &summary, &meta, &sets) {
        Ok(kind) => println!("classify_via_swatches: {:?}", kind),
        Err(e) => println!("classify error: {e:?}"),
    }
    match palette::debug_swatches(&data, &plain, &summary, &meta, &sets) {
        Ok(v) => {
            for sw in v {
                println!(
                    "{}: hue={} value={} color_count={} payload_len={} first_bytes={:02x?}",
                    sw.0, sw.1, sw.2, sw.3, sw.4, &sw.5[..sw.5.len().min(16)]
                );
            }
        }
        Err(e) => println!("swatches error: {e:?}"),
    }
}
