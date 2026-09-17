use std::env;
use std::path::Path;

fn main() {
    let dir: String = env::args()
        .nth(1)
        .unwrap_or_else(|| r"E:\games\rocketleague\TAGame\CookedPCConsole".into());
    let cooked = Path::new(&dir);
    println!("{}", app_lib::upk::palette::probe_engine_refs(cooked));
    match app_lib::upk::palette::repair_tagame_engine_refs(cooked) {
        Ok(msg) => println!("repair: {msg}"),
        Err(e) => println!("repair ERR: {e}"),
    }
    println!("--- after ---");
    println!("{}", app_lib::upk::palette::probe_engine_refs(cooked));
}
