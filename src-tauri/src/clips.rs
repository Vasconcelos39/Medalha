//! Biblioteca local de clipes: listagem, miniaturas, renomear, lixeira.

use crate::util::{command, partial_path, run_with_timeout, unique_path};
use crate::{media, sound, AppState};
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
        let Some(out) = run_with_timeout(cmd, Duration::from_secs(20)) else { break };
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

/// Garante que `path` está dentro da pasta de clipes. Devolve o caminho como veio:
/// no Windows o canônico tem o prefixo `\\?\`, que o Explorer não entende.
pub fn ensure_inside(dir: &Path, path: &str) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let canonical = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
    if canonical.starts_with(&dir) && is_video(&canonical) && Path::new(path).is_absolute() {
        Ok(PathBuf::from(path))
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
    // No Windows, mudar só maiúsculas/minúsculas aponta para o mesmo arquivo.
    let same_file = target.canonicalize().ok().is_some_and(|t| path.canonicalize().ok() == Some(t));
    if target.exists() && !same_file {
        return Err("Já existe um clipe com esse nome.".into());
    }
    std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Corta `[start, end]` (segundos) num clipe novo ao lado do original, sem reencodar.
/// Com `-c copy` o corte começa no keyframe anterior a `start` (o buffer do ffmpeg
/// grava um por segundo).
pub fn trim(ffmpeg: &Path, clip: &Path, start: f64, end: f64) -> Result<ClipInfo, String> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end - start < 0.5 {
        return Err("Escolha um trecho de pelo menos meio segundo.".into());
    }
    let dir = clip.parent().ok_or("Caminho inválido.")?;
    let stem = clip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = clip.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let out = unique_path(dir, &format!("{stem} (corte).{ext}"));
    let partial = partial_path(&out);
    let muxer = match ext.as_str() {
        "mkv" => "matroska",
        "webm" => "webm",
        "mov" => "mov",
        _ => "mp4",
    };

    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-ss"])
        .arg(format!("{start:.3}"))
        .arg("-i")
        .arg(clip)
        .arg("-t")
        .arg(format!("{:.3}", end - start))
        .args(["-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero"]);
    if muxer == "mp4" || muxer == "mov" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.args(["-f", muxer]).arg(&partial);
    let output = run_with_timeout(cmd, Duration::from_secs(120)).ok_or("O ffmpeg demorou demais para cortar o clipe.")?;
    if !output.status.success() || partial.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("Falha ao cortar: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    std::fs::rename(&partial, &out).map_err(|e| e.to_string())?;
    info(&out).ok_or_else(|| "O corte foi salvo, mas não consegui lê-lo.".into())
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

/// Depois de salvar: toca o som, gera a miniatura, avisa a UI e mostra o aviso.
pub fn on_clip_saved(app: &AppHandle, path: PathBuf) {
    crate::logfile::write(format!("clipe salvo: {}", path.display()));
    let state = app.state::<AppState>();
    let (ffmpeg_path, beep) = {
        let cfg = state.config.lock().unwrap();
        (cfg.ffmpeg_path.clone(), cfg.sound_on_save)
    };
    if beep {
        sound::play_saved(&state.cache_dir);
    }
    let ffmpeg = media::find_ffmpeg(&ffmpeg_path);
    let meta = meta(ffmpeg.as_deref(), &state.cache_dir, &path);
    let Some(clip) = info(&path) else { return };
    let title = clip_title(&clip.name);
    let subtitle = match meta.duration {
        Some(secs) => format!("{title} · {secs:.0} s"),
        None => title,
    };
    alert(app, "Clipe salvo", &subtitle, "ok");
    let _ = app.emit("clip-saved", clip);
}

/// Aviso ao salvar (ou ao falhar): card por cima do jogo e, se não der, notificação
/// do sistema. Chame fora da thread principal.
pub fn alert(app: &AppHandle, title: &str, text: &str, kind: &str) {
    let (enabled, overlay) = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        (cfg.notify_on_save, cfg.overlay_on_save)
    };
    if !enabled {
        return;
    }
    let short: String = text.lines().next().unwrap_or_default().chars().take(90).collect();
    if overlay && crate::overlay::show(app, title, &short, kind) {
        return;
    }
    let _ = app.notification().builder().title(title).body(short).show();
}

/// `Elden Ring_2026-10-23_21-00-00` → `Elden Ring`; sem jogo no nome, "Replay".
fn clip_title(stem: &str) -> String {
    let mut parts = stem.rsplitn(3, '_');
    let (time, date, prefix) = (parts.next(), parts.next(), parts.next());
    let dated = date.is_some_and(|d| d.len() == 10 && d.as_bytes()[4] == b'-')
        && time.is_some_and(|t| t.starts_with(|c: char| c.is_ascii_digit()));
    match prefix {
        Some("Medalha") if dated => "Replay".into(),
        Some(game) if dated => game.into(),
        _ => stem.into(),
    }
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

#[cfg(test)]
mod tests {
    use super::clip_title;

    #[test]
    fn titles() {
        assert_eq!(clip_title("Elden Ring_2026-10-23_21-00-00"), "Elden Ring");
        assert_eq!(clip_title("Medalha_2026-10-23_21-00-00"), "Replay");
        assert_eq!(clip_title("Elden Ring_2026-10-23_21-00-00 (2)"), "Elden Ring");
        assert_eq!(clip_title("meu clipe"), "meu clipe");
        assert_eq!(clip_title("a_b_c"), "a_b_c");
    }
}
