//! Tiny logger: appends to `logs\launcher.log`, optionally echoes to stdout (CLI mode).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct State {
    file: Option<File>,
    echo: bool,
}

static STATE: Mutex<State> = Mutex::new(State { file: None, echo: false });

pub fn init(path: &Path, echo: bool) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    // Keep the log from growing forever: start fresh when it is over 1 MB.
    if std::fs::metadata(path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(path);
    }
    let file = OpenOptions::new().create(true).append(true).open(path).ok();
    if let Ok(mut s) = STATE.lock() {
        s.file = file;
        s.echo = echo;
    }
}

pub fn info(msg: impl AsRef<str>) {
    write("INFO", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    write("WARN", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    write("ERROR", msg.as_ref());
}

/// Output meant for the person running the CLI: printed to stdout and also logged.
pub fn out(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if let Ok(mut s) = STATE.lock() {
        if let Some(f) = s.file.as_mut() {
            let _ = writeln!(f, "{} {}", timestamp(), msg);
        }
    }
    let mut so = std::io::stdout();
    let _ = writeln!(so, "{msg}");
    let _ = so.flush();
}

/// Printed for the person at the console only, never written to launcher.log
/// (used for the sign-in code).
pub fn console(msg: impl AsRef<str>) {
    let mut so = std::io::stdout();
    let _ = writeln!(so, "{}", msg.as_ref());
    let _ = so.flush();
}

/// Tests: release the log file so it can be read and deleted.
#[cfg(test)]
pub fn close() {
    if let Ok(mut s) = STATE.lock() {
        s.file = None;
    }
}

fn write(level: &str, msg: &str) {
    let Ok(mut s) = STATE.lock() else { return };
    if let Some(f) = s.file.as_mut() {
        let _ = writeln!(f, "{} [{}] {}", timestamp(), level, msg);
    }
    if s.echo {
        let mut so = std::io::stdout();
        let _ = writeln!(so, "[{level}] {msg}");
        let _ = so.flush();
    }
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}
