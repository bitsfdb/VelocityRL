use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, RunEvent};

const LAUNCH_KEEP: usize = 5;
const CRASH_KEEP: usize = 5;

static LOG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;
static PREVIOUS_HOOK: Mutex<Option<PanicHook>> = Mutex::new(None);

fn format_utc(secs: u64) -> String {

    let z = (secs / 86400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let tod = secs % 86400;
    let h = tod / 3600;
    let min = (tod % 3600) / 60;
    let s = tod % 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}Z")
}

fn format_utc_compact(secs: u64) -> String {
    format_utc(secs)
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_stamp() -> String {
    format_utc(now_secs())
}

fn crash_file_stamp() -> String {
    format_utc_compact(now_secs())
}

fn resolve_logs_dir(app: &AppHandle) -> PathBuf {
    if let Ok(dir) = app.path().app_log_dir() {
        return dir;
    }
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
}

fn rotate_numbered(dir: &Path, stem: &str, keep: usize) {
    let newest = dir.join(format!("{stem}.log"));
    let last = dir.join(format!("{stem}.{keep}.log"));
    let _ = fs::remove_file(&last);
    for i in (1..keep).rev() {
        let from = dir.join(format!("{stem}.{i}.log"));
        let to = dir.join(format!("{stem}.{}.log", i + 1));
        let _ = fs::rename(&from, &to);
    }
    if newest.is_file() {
        let _ = fs::rename(&newest, dir.join(format!("{stem}.1.log")));
    }
}

fn prune_crash_logs(dir: &Path, keep: usize) {
    let mut crashes: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("crash-") && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    crashes.sort();
    let excess = crashes.len().saturating_sub(keep);
    for p in crashes.into_iter().take(excess) {
        let _ = fs::remove_file(p);
    }
}

fn session_marker(dir: &Path) -> PathBuf {
    dir.join("session.active")
}

fn append_raw(path: &Path, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
    }
}

pub fn init(app: &AppHandle) -> PathBuf {
    let dir = resolve_logs_dir(app);
    let _ = fs::create_dir_all(&dir);

    let unclean = session_marker(&dir).is_file();
    rotate_numbered(&dir, "launch", LAUNCH_KEEP);
    prune_crash_logs(&dir, CRASH_KEEP);

    let launch = dir.join("launch.log");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let header = format!(
        "=== VelocityRL launch ===\n\
         time: {}\n\
         version: {}\n\
         profile: {}\n\
         os: {}-{}\n\
         pid: {}\n\
         log_dir: {}\n\
         previous_exit_clean: {}\n\
         ---",
        now_stamp(),
        env!("CARGO_PKG_VERSION"),
        profile,
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id(),
        dir.display(),
        !unclean
    );
    let _ = fs::write(&launch, format!("{header}\n"));
    if unclean {
        append_raw(
            &launch,
            &format!(
                "[{}] WARN previous session left session.active (possible crash / ACCESS_VIOLATION / kill)",
                now_stamp()
            ),
        );
        let hint = dir.join(format!("crash-{}-unclean.log", crash_file_stamp()));
        let note = format!(
            "Detected unclean previous exit at next launch {}\n\
             (STATUS_ACCESS_VIOLATION / hard kill may not hit Rust panic hooks)\n\
             Marker was: {}\n",
            now_stamp(),
            session_marker(&dir).display()
        );
        let _ = fs::write(&hint, note);
        prune_crash_logs(&dir, CRASH_KEEP);
    }

    let _ = fs::write(
        session_marker(&dir),
        format!(
            "pid={}\nstarted={}\nversion={}\n",
            std::process::id(),
            now_stamp(),
            env!("CARGO_PKG_VERSION")
        ),
    );

    if let Ok(mut guard) = LOG_DIR.lock() {
        *guard = Some(dir.clone());
    }

    install_panic_hook();

    log::info!("launch log: {}", launch.display());
    dir
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    if let Ok(mut g) = PREVIOUS_HOOK.lock() {
        *g = Some(prev);
    }

    std::panic::set_hook(Box::new(|info| {
        write_panic_crash(info);
        if let Ok(g) = PREVIOUS_HOOK.lock() {
            if let Some(ref prev) = *g {
                prev(info);
            }
        }
    }));
}

fn write_panic_crash(info: &std::panic::PanicHookInfo<'_>) {
    let dir = LOG_DIR.lock().ok().and_then(|g| g.clone());
    let Some(dir) = dir else { return };

    let path = dir.join(format!("crash-{}.log", crash_file_stamp()));
    let mut body = String::new();
    body.push_str("=== VelocityRL crash (panic) ===\n");
    body.push_str(&format!("time: {}\n", now_stamp()));
    body.push_str(&format!("version: {}\n", env!("CARGO_PKG_VERSION")));
    body.push_str(&format!("pid: {}\n", std::process::id()));
    body.push_str(&format!("{info}\n"));
    if let Some(loc) = info.location() {
        body.push_str(&format!(
            "location: {}:{}:{}\n",
            loc.file(),
            loc.line(),
            loc.column()
        ));
    }
    let _ = fs::write(&path, body);
    prune_crash_logs(&dir, CRASH_KEEP);
    event(&format!("PANIC written to {}", path.display()));
}

pub fn event(message: &str) {
    let line = format!("[{}] {}", now_stamp(), message);
    log::info!("{message}");
    if let Ok(guard) = LOG_DIR.lock() {
        if let Some(ref dir) = *guard {
            append_raw(&dir.join("launch.log"), &line);
        }
    }
}

pub fn mark_clean_exit() {
    if let Ok(guard) = LOG_DIR.lock() {
        if let Some(ref dir) = *guard {
            event("clean exit");
            let _ = fs::remove_file(session_marker(dir));
        }
    }
}

pub fn on_run_event(_app: &AppHandle, ev: &RunEvent) {
    match ev {
        RunEvent::Exit => {
            mark_clean_exit();
        }
        RunEvent::ExitRequested { .. } => {
            event("exit requested");
        }
        _ => {}
    }
}

#[tauri::command]
pub fn append_launch_log(message: String) -> Result<(), String> {
    event(&message);
    Ok(())
}

#[tauri::command]
pub fn get_logs_dir(app: AppHandle) -> Result<String, String> {
    let dir = resolve_logs_dir(&app);
    let _ = fs::create_dir_all(&dir);
    Ok(dir.to_string_lossy().into_owned())
}
