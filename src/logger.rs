use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

const MAX_LOG_SIZE: u64 = 1_048_576;
static LOG: OnceLock<Mutex<File>> = OnceLock::new();

pub fn init() -> Result<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("cannot determine state directory")?;
    let directory = base.join("game-translate");
    fs::create_dir_all(&directory).context("cannot create log directory")?;
    let path = directory.join("game-translate.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .context("cannot open log file")?;
    if file.metadata()?.len() > MAX_LOG_SIZE {
        file.set_len(0)?;
    }
    let _ = LOG.set(Mutex::new(file));
    write("session", "started");
    Ok(path)
}

pub fn write(kind: &str, message: &str) {
    let Some(log) = LOG.get() else { return };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if let Ok(mut file) = log.lock() {
        let sanitized = message.replace(['\n', '\r'], " ");
        let _ = writeln!(file, "{millis}\t{kind}\t{sanitized}");
    }
}
