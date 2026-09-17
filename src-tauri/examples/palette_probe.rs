use std::env;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let dir = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(Path::new)
        .unwrap_or_else(|| Path::new(r"E:\games\rocketleague\TAGame\CookedPCConsole"));
    let do_apply = args.iter().any(|a| a == "--apply");
    let do_restore = args.iter().any(|a| a == "--restore");
    let do_dump = args.iter().any(|a| a == "--dump");
    let do_dump_orig = args.iter().any(|a| a == "--dump-orig");
    let keys = include_str!("../resources/keys.txt");
    let keymap = include_str!("../resources/keys_map.json");

    if do_dump || do_dump_orig {
        let st = app_lib::upk::palette::status(dir, None);
        let path = if do_dump_orig {
            std::path::PathBuf::from(&st.backup_path)
        } else {
            std::path::PathBuf::from(&st.tagame_path)
        };
        match app_lib::upk::palette::dump_color_sets(&path, keys, keymap) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("DUMP ERR: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let st = app_lib::upk::palette::status(dir, None);
    println!(
        "status applied={} backup={} msg {}\n  tagame={}\n  backup={}",
        st.applied, st.backup_present, st.message, st.tagame_path, st.backup_path
    );

    if do_restore {
        match app_lib::upk::palette::restore(dir) {
            Ok(st) => println!("RESTORE OK applied={} msg={}", st.applied, st.message),
            Err(e) => {
                println!("RESTORE ERR: {e}");
                std::process::exit(1);
            }
        }
    } else if do_apply {
        match app_lib::upk::palette::apply(dir, keys, keymap) {
            Ok(st) => println!(
                "APPLY OK applied={} msg={} fp={}",
                st.applied, st.message, st.fingerprint
            ),
            Err(e) => {
                println!("APPLY ERR: {e}");
                std::process::exit(1);
            }
        }
    }

    let st = app_lib::upk::palette::status(dir, None);
    println!("after status applied={} msg={}", st.applied, st.message);
}
