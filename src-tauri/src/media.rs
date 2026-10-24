//! Descoberta do ffmpeg e escolha do pipeline de captura/encoder.

use crate::config::Config;
use crate::util::{command, find_program, run_with_timeout};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(target_os = "linux")]
pub const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

pub fn find_ffmpeg(custom: &str) -> Option<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        return p.is_file().then_some(p);
    }
    let exe_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    // ffmpeg empacotado ao lado do executável tem prioridade.
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join(exe_name);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = find_program("ffmpeg") {
        return Some(p);
    }
    #[cfg(windows)]
    if let Some(p) = find_winget_ffmpeg() {
        return Some(p);
    }
    None
}

/// O winget põe o ffmpeg no PATH do usuário, mas processos abertos antes da
/// instalação herdam o PATH antigo; então procuramos direto nas pastas dele.
#[cfg(windows)]
fn find_winget_ffmpeg() -> Option<PathBuf> {
    let winget = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Microsoft\\WinGet");
    let link = winget.join("Links\\ffmpeg.exe");
    if link.is_file() {
        return Some(link);
    }
    let packages = std::fs::read_dir(winget.join("Packages")).ok()?;
    packages
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().contains("ffmpeg"))
        .flat_map(|pkg| {
            let direct = pkg.path().join("bin\\ffmpeg.exe");
            let nested = std::fs::read_dir(pkg.path())
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path().join("bin\\ffmpeg.exe"));
            std::iter::once(direct).chain(nested).collect::<Vec<_>>()
        })
        .find(|p| p.is_file())
}

fn ffmpeg_ok(ffmpeg: &Path, args: &[&str], timeout_secs: u64) -> bool {
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]).args(args);
    run_with_timeout(cmd, Duration::from_secs(timeout_secs))
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Lista de dispositivos de entrada (`ffmpeg -devices`), ex.: x11grab, pulse.
#[cfg_attr(windows, allow(dead_code))]
pub fn has_device(ffmpeg: &Path, name: &str) -> bool {
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-devices"]);
    run_with_timeout(cmd, Duration::from_secs(10))
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name))
        })
        .unwrap_or(false)
}

/// Testa um encoder de verdade (a lista do `-encoders` não garante que a GPU exista).
pub fn encoder_works(ffmpeg: &Path, encoder: &str) -> bool {
    let mut args: Vec<&str> = Vec::new();
    #[cfg(target_os = "linux")]
    if encoder == "h264_vaapi" {
        args.extend(["-vaapi_device", VAAPI_DEVICE]);
    }
    args.extend(["-f", "lavfi", "-i", "color=c=black:s=1280x720:r=30", "-frames:v", "5"]);
    match encoder {
        "h264_vaapi" => args.extend(["-vf", "format=nv12,hwupload"]),
        "libx264" => args.extend(["-pix_fmt", "yuv420p"]),
        _ => args.extend(["-pix_fmt", "nv12"]),
    }
    args.extend(["-c:v", encoder, "-f", "null", "-"]);
    ffmpeg_ok(ffmpeg, &args, 15)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum Capture {
    /// Desktop Duplication API (Windows 8+), via filtro lavfi.
    Ddagrab,
    Gdigrab,
    X11grab,
}

#[derive(Clone, Debug)]
pub struct VideoPipeline {
    pub capture: Capture,
    pub encoder: String,
    /// Frames D3D11 vão direto pro encoder de GPU, sem passar pela CPU.
    pub direct_hw: bool,
    /// ffmpeg tem entrada PulseAudio (Linux).
    #[cfg_attr(windows, allow(dead_code))]
    pub pulse: bool,
}

impl VideoPipeline {
    pub fn describe(&self) -> String {
        let cap = match self.capture {
            Capture::Ddagrab => "ddagrab",
            Capture::Gdigrab => "gdigrab",
            Capture::X11grab => "x11grab",
        };
        let path = if self.direct_hw { " (zero-copy)" } else { "" };
        format!("{cap} → {}{path}", self.encoder)
    }
}

fn encoder_candidates(cfg: &Config) -> Vec<String> {
    let auto: &[&str] = if cfg!(windows) {
        &["h264_nvenc", "h264_amf", "h264_qsv", "libx264"]
    } else {
        &["h264_nvenc", "h264_vaapi", "libx264"]
    };
    if cfg.encoder == "auto" || cfg.encoder.is_empty() {
        auto.iter().map(|s| s.to_string()).collect()
    } else {
        vec![cfg.encoder.clone(), "libx264".into()]
    }
}

/// Chave de cache: o probe só precisa rodar de novo se isso mudar.
pub fn pipeline_key(ffmpeg: &Path, cfg: &Config) -> String {
    format!("{}|{}|{}|{}", ffmpeg.display(), cfg.encoder, cfg.resolution, cfg.monitor)
}

pub fn resolve_pipeline(ffmpeg: &Path, cfg: &Config) -> Result<VideoPipeline, String> {
    #[cfg(windows)]
    {
        let dda_input = format!("ddagrab=output_idx={}:framerate=10", cfg.monitor);
        let dda_ok = ffmpeg_ok(
            ffmpeg,
            &["-f", "lavfi", "-i", &dda_input, "-frames:v", "1", "-vf", "hwdownload,format=bgra", "-f", "null", "-"],
            10,
        );
        let capture = if dda_ok { Capture::Ddagrab } else { Capture::Gdigrab };
        for enc in encoder_candidates(cfg) {
            let can_direct = capture == Capture::Ddagrab
                && cfg.max_height().is_none()
                && (enc == "h264_nvenc" || enc == "h264_amf");
            if can_direct
                && ffmpeg_ok(
                    ffmpeg,
                    &["-f", "lavfi", "-i", &dda_input, "-frames:v", "3", "-c:v", &enc, "-f", "null", "-"],
                    10,
                )
            {
                return Ok(VideoPipeline { capture, encoder: enc, direct_hw: true, pulse: false });
            }
            if encoder_works(ffmpeg, &enc) {
                return Ok(VideoPipeline { capture, encoder: enc, direct_hw: false, pulse: false });
            }
        }
        Err("Nenhum encoder H.264 funcionou no ffmpeg.".into())
    }
    #[cfg(not(windows))]
    {
        if !has_device(ffmpeg, "x11grab") {
            return Err("Este ffmpeg não tem x11grab. Instale o ffmpeg da sua distro.".into());
        }
        let pulse = has_device(ffmpeg, "pulse");
        for enc in encoder_candidates(cfg) {
            if encoder_works(ffmpeg, &enc) {
                return Ok(VideoPipeline { capture: Capture::X11grab, encoder: enc, direct_hw: false, pulse });
            }
        }
        Err("Nenhum encoder H.264 funcionou no ffmpeg.".into())
    }
}

/// Lê a duração (segundos) da saída de `ffmpeg -i`.
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let rest = &stderr[stderr.find("Duration: ")? + 10..];
    let stamp = rest.split(',').next()?.trim();
    let mut parts = stamp.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}
