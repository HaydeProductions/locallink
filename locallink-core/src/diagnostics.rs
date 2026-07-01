use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn log(component: &str, msg: impl AsRef<str>) {
    if std::env::var("LOCALLINK_DIAGNOSTICS")
        .map(|value| value == "0" || value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return;
    }

    let path = log_path("diagnostics.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let line = format!(
        "[{}] [{}] pid={} {}\n",
        now_ms(),
        component,
        std::process::id(),
        msg.as_ref()
    );

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn log_path(filename: &str) -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    base.join("LocalLink").join("logs").join(filename)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
