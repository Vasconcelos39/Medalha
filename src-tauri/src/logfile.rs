//! `medalha.log` na pasta de cache. Cada linha vai para o disco na hora (sync), para
//! que um travamento do computador não leve junto o que aconteceu antes dele.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Acima disso, o log anterior vira `medalha.log.old` quando o app abre.
const MAX_BYTES: u64 = 1_000_000;

static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("medalha.log")
}

pub fn init(cache_dir: &Path) {
    let path = path(cache_dir);
    let _ = std::fs::create_dir_all(cache_dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, cache_dir.join("medalha.log.old"));
    }
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = OpenOptions::new().create(true).append(true).open(&path).ok();
}

pub fn write(message: impl AsRef<str>) {
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = file.as_mut() else { return };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let _ = writeln!(f, "{now}  {}", message.as_ref());
    let _ = f.sync_data();
}
