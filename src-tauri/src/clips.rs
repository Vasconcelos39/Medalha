//! Biblioteca local de clipes: listagem, miniaturas, renomear, lixeira.

use crate::util::command;
use crate::{media, AppState};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "mov"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
    /// ms desde a época
    pub modified: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    pub thumb: Option<String>,
    pub duration: Option<f64>,
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn info(path: &Path) -> Option<ClipInfo> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || !is_video(path) {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some(ClipInfo {
        path: path.to_string_lossy().into_owned(),
        name: path.file_stem()?.to_string_lossy().into_owned(),
        size: meta.len(),
        modified,
    })
}

pub fn list(dir: &Path) -> Vec<ClipInfo> {
    let mut clips: Vec<ClipInfo> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| info(&e.path())).collect())
        .unwrap_or_default();
    clips.sort_by(|a, b| b.modified.cmp(&a.modified));
    clips
}

/// Miniatura + duração, em cache por (caminho, tamanho, data).
pub fn meta(ffmpeg: Option<&Path>, cache_dir: &Path, clip: &Path) -> ClipMeta {
    let Some(info) = info(clip) else { return ClipMeta::default() };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&info.path, info.size, info.modified).hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let thumbs = cache_dir.join("thumbs");
    let thumb = thumbs.join(format!("{key}.jpg"));
    let json = thumbs.join(format!("{key}.json"));
    if let Some(cached) = std::fs::read_to_string(&json).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        return cached;
    }
    let Some(ffmpeg) = ffmpeg else { return ClipMeta::default() };
    let _ = std::fs::create_dir_all(&thumbs);

    let mut duration = None;
    // Pula o primeiro segundo (costuma ser preto); clipes curtos usam o frame 0.
    for seek in ["1", "0"] {
        let mut cmd = command(ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-y", "-ss", seek, "-i"])
            .arg(clip)
            .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4"])
            .arg(&thumb);
        let Some(out) = crate::util::run_with_timeout(cmd, Duration::from_secs(20)) else { break };
        duration = duration.or_else(|| media::parse_duration(&String::from_utf8_lossy(&out.stderr)));
        if thumb.is_file() {
            break;
        }
    }
    let meta = ClipMeta {
        thumb: thumb.is_file().then(|| thumb.to_string_lossy().into_owned()),
        duration,
    };
    if meta.thumb.is_some() {
        let _ = std::fs::write(&json, serde_json::to_string(&meta).unwrap_or_default());
    }
    meta
}

/// Garante que `path` está dentro da pasta de clipes.
pub fn ensure_inside(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let path = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
    if path.starts_with(&dir) && is_video(&path) {
        Ok(path)
    } else {
        Err("Arquivo fora da pasta de clipes.".into())
    }
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let clean: String = new_name
        .trim()
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if clean.is_empty() {
        return Err("Nome inválido.".into());
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let target = path.with_file_name(format!("{clean}.{ext}"));
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err("Já existe um clipe com esse nome.".into());
    }
    std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// Abre o gerenciador de arquivos com o clipe selecionado.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = command("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let uri = format!("file://{}", percent_encode(&path.to_string_lossy()));
        let shown = command("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
            ])
            .arg(format!("array:string:{uri}"))
            .arg("string:")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !shown {
            if let Some(dir) = path.parent() {
                open_path(dir);
            }
        }
    }
}

/// Abre um arquivo/pasta com o programa padrão do sistema.
pub fn open_path(path: &Path) {
    #[cfg(windows)]
    let _ = command("explorer.exe").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = command("xdg-open").arg(path).spawn();
}

#[cfg(not(windows))]
fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Depois de salvar: gera a miniatura, avisa a UI e mostra notificação.
pub fn on_clip_saved(app: &AppHandle, path: PathBuf) {
    let state = app.state::<AppState>();
    let (ffmpeg_path, notify) = {
        let cfg = state.config.lock().unwrap();
        (cfg.ffmpeg_path.clone(), cfg.notify_on_save)
    };
    let ffmpeg = media::find_ffmpeg(&ffmpeg_path);
    meta(ffmpeg.as_deref(), &state.cache_dir, &path);
    let Some(clip) = info(&path) else { return };
    if notify {
        let _ = app
            .notification()
            .builder()
            .title("Clipe salvo")
            .body(&clip.name)
            .show();
    }
    let _ = app.emit("clip-saved", clip);
}

/// Observa a pasta de clipes e avisa a UI quando algo muda.
pub fn watch(app: &AppHandle, dir: &Path) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let app = app.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = app.emit("clips-changed", ());
        }
    })
    .ok()?;
    watcher.watch(dir, notify::RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}
