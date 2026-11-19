//! Build a TAGame.upk with the avatar + sidecar-URL patch already applied.
//! Usage: cargo run --example make_avatar_upk -- <avatar.png> <stock TAGame.upk> <output TAGame.upk>

use std::{env, fs, path::Path};

fn main() {
    let args: Vec<String> = env::args().collect();
    let image = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("/root/rl_japan_1024x1024_center_crop.png");
    let stock = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("/root/velrlapi/.cache/TAGame.upk.bak");
    let out = args
        .get(3)
        .map(String::as_str)
        .unwrap_or("/root/TAGame_avatar_patched.upk");

    let image_bytes = fs::read(image).unwrap_or_else(|e| panic!("read {image}: {e}"));
    let (bgra, _) = app_lib::avatar::process_avatar_image(&image_bytes)
        .unwrap_or_else(|e| panic!("process {image}: {e}"));

    fs::copy(stock, out).unwrap_or_else(|e| panic!("copy {stock} -> {out}: {e}"));
    app_lib::avatar::apply_custom_avatar_to_tagame(Path::new(out), &bgra)
        .unwrap_or_else(|e| panic!("patch {out}: {e}"));
    println!("wrote patched UPK: {out}");
}
