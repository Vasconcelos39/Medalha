//! Replay buffer com ffmpeg: grava continuamente em segmentos MPEG-TS de 2 s
//! (reaproveitados em ciclo) e, ao salvar, junta os últimos N segundos num MP4
//! sem reencodar.

use crate::config::Config;
use crate::media::{Capture, VideoPipeline};
use crate::util::{command, join_tail, kill_with_parent, partial_path, run_with_timeout, tail_reader};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const SEGMENT_SECS: u32 = 2;

pub struct FfmpegSession {
    child: Child,
    pub saver: Saver,
    stderr: Arc<Mutex<VecDeque<String>>>,
    #[cfg(windows)]
    audio: Option<super::audio_win::AudioPump>,
}

#[derive(Clone)]
pub struct Saver {
    ffmpeg: PathBuf,
    buffer_dir: PathBuf,
}

impl FfmpegSession {
    pub fn start(
        ffmpeg: &Path,
        pipeline: &VideoPipeline,
        cfg: &Config,
        buffer_dir: &Path,
    ) -> Result<Self, String> {
        let _ = std::fs::remove_dir_all(buffer_dir);
        std::fs::create_dir_all(buffer_dir).map_err(|e| format!("buffer: {e}"))?;

        let (args, uses_stdin) = build_args(pipeline, cfg, buffer_dir);
        let mut child = command(ffmpeg)
            .args(&args)
            .stdin(if uses_stdin { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("não consegui iniciar o ffmpeg: {e}"))?;
        kill_with_parent(&child);
        let stderr = tail_reader(child.stderr.take().expect("stderr"), 12, None);

        #[cfg(windows)]
        let audio = child
            .stdin
            .take()
            .map(|stdin| super::audio_win::AudioPump::start(stdin, cfg.system_audio, cfg.mic));

        Ok(Self {
            child,
            saver: Saver { ffmpeg: ffmpeg.to_path_buf(), buffer_dir: buffer_dir.to_path_buf() },
            stderr,
            #[cfg(windows)]
            audio,
        })
    }

    /// `Some(erro)` se o ffmpeg morreu.
    pub fn poll_exit(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let tail = join_tail(&self.stderr);
                Some(if tail.is_empty() { format!("ffmpeg saiu ({status})") } else { tail })
            }
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        if let Some(audio) = self.audio.take() {
            audio.stop();
        }
        let _ = std::fs::remove_dir_all(&self.saver.buffer_dir);
    }
}

/// Segmentos com dados, do mais antigo ao mais novo (pela data de modificação).
fn segments(dir: &Path) -> Vec<(SystemTime, PathBuf)> {
    let mut segments: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "ts"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            (meta.len() > 0).then(|| (meta.modified().ok().unwrap_or(SystemTime::UNIX_EPOCH), e.path()))
        })
        .collect();
    segments.sort();
    segments
}

impl Saver {
    /// Quantas vezes mais devagar que o tempo real o ffmpeg está gravando: o tempo
    /// que os últimos segmentos levaram dividido pelo que eles deveriam cobrir.
    /// Passa de 1 quando a máquina não dá conta do fps configurado.
    pub fn slowdown(&self) -> Option<f64> {
        let mut times: Vec<SystemTime> = segments(&self.buffer_dir).into_iter().map(|(t, _)| t).collect();
        times.pop(); // o mais novo ainda está aberto
        let recent = &times[times.len().saturating_sub(6)..];
        if recent.len() < 6 {
            return None;
        }
        let wall = recent.last()?.duration_since(recent[0]).ok()?.as_secs_f64();
        Some(wall / ((recent.len() - 1) as f64 * SEGMENT_SECS as f64))
    }

    pub fn save(&self, out: &Path, secs: u32) -> Result<(), String> {
        let segments = segments(&self.buffer_dir);
        if segments.is_empty() {
            return Err("O buffer ainda está vazio, espere alguns segundos.".into());
        }
        // A data de modificação de cada segmento é o fim dele: o primeiro que termina
        // dentro da janela já contém o começo dela. Contar segmentos não dá certo,
        // porque o keyframe vem a cada N frames e, quando o ffmpeg perde frames, um
        // segmento passa de 2 s e o clipe sai mais longo que o pedido.
        let newest = segments.last().map(|(t, _)| *t).unwrap_or(SystemTime::UNIX_EPOCH);
        let window = Duration::from_secs(secs.into());
        let first = segments
            .iter()
            .position(|(t, _)| newest.duration_since(*t).map(|age| age < window).unwrap_or(true))
            .unwrap_or(0);
        let chosen = &segments[first..];

        let list: String = chosen
            .iter()
            .map(|(_, p)| {
                let p = p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
                format!("file '{p}'\n")
            })
            .collect();
        let list_path = self.buffer_dir.with_extension("concat.txt");
        std::fs::write(&list_path, list).map_err(|e| e.to_string())?;

        let partial = partial_path(out);
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list_path)
            .args(["-map", "0", "-c", "copy", "-movflags", "+faststart", "-f", "mp4"])
            .arg(&partial);
        let result = run_with_timeout(cmd, Duration::from_secs(120));
        let _ = std::fs::remove_file(&list_path);
        let output = result.ok_or("ffmpeg demorou demais para salvar o clipe")?;
        let ok = output.status.success() && partial.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if ok {
            std::fs::rename(&partial, out).map_err(|e| e.to_string())
        } else {
            let _ = std::fs::remove_file(&partial);
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }
}

fn s(v: impl ToString) -> String {
    v.to_string()
}

fn build_args(pipeline: &VideoPipeline, cfg: &Config, buffer_dir: &Path) -> (Vec<String>, bool) {
    let fps = cfg.fps;
    let fps_s = fps.to_string();
    let mut args: Vec<String> = vec![s("-hide_banner"), s("-loglevel"), s("error")];

    #[cfg(target_os = "linux")]
    if pipeline.encoder == "h264_vaapi" {
        args.extend([s("-vaapi_device"), s(crate::media::VAAPI_DEVICE)]);
    }

    // Vídeo (entrada 0)
    match pipeline.capture {
        Capture::Ddagrab => args.extend([
            s("-f"),
            s("lavfi"),
            s("-i"),
            format!("ddagrab=output_idx={}:framerate={fps}:draw_mouse=1", cfg.monitor),
        ]),
        Capture::Gdigrab => args.extend(
            ["-f", "gdigrab", "-thread_queue_size", "1024", "-framerate", fps_s.as_str(), "-draw_mouse", "1", "-i", "desktop"]
                .map(s),
        ),
        Capture::X11grab => {
            let (input, size) = x11_target(cfg.monitor);
            args.extend(["-f", "x11grab", "-thread_queue_size", "1024", "-framerate", fps_s.as_str(), "-draw_mouse", "1"].map(s));
            if let Some(size) = size {
                args.extend([s("-video_size"), size]);
            }
            args.extend([s("-i"), input]);
        }
    }

    // Áudio (entradas 1..)
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut audio_inputs = 0;
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut uses_stdin = false;
    #[cfg(windows)]
    if cfg.system_audio || cfg.mic {
        let rate = super::audio_win::RATE.to_string();
        args.extend(["-f", "f32le", "-ar", rate.as_str(), "-ac", "2", "-thread_queue_size", "1024", "-i", "pipe:0"].map(s));
        audio_inputs = 1;
        uses_stdin = true;
    }
    #[cfg(target_os = "linux")]
    if pipeline.pulse {
        for source in pulse_sources(cfg) {
            args.extend(["-f", "pulse", "-thread_queue_size", "1024", "-i", source.as_str()].map(s));
            audio_inputs += 1;
        }
    }
    if !uses_stdin {
        args.insert(0, s("-nostdin"));
    }

    args.extend([s("-map"), s("0:v")]);
    match audio_inputs {
        0 => {}
        1 => args.extend([s("-map"), s("1:a")]),
        n => {
            let inputs: String = (1..=n).map(|i| format!("[{i}:a]")).collect();
            args.extend([
                s("-filter_complex"),
                format!("{inputs}amix=inputs={n}:duration=longest:normalize=0[aout]"),
                s("-map"),
                s("[aout]"),
            ]);
        }
    }

    if let Some(vf) = video_filter(pipeline, cfg) {
        args.extend([s("-vf"), vf]);
    }

    let kbps = cfg.video_kbps();
    let bitrate = format!("{kbps}k");
    let bufsize = format!("{}k", kbps * 2);
    args.extend([s("-c:v"), pipeline.encoder.clone()]);
    match pipeline.encoder.as_str() {
        "libx264" => args.extend(["-preset", "veryfast", "-b:v", bitrate.as_str(), "-maxrate", bitrate.as_str(), "-bufsize", bufsize.as_str()].map(s)),
        "h264_nvenc" => args.extend(["-preset", "p4", "-rc", "cbr", "-b:v", bitrate.as_str()].map(s)),
        "h264_amf" => args.extend(["-quality", "speed", "-rc", "cbr", "-b:v", bitrate.as_str()].map(s)),
        "h264_qsv" => args.extend(["-preset", "veryfast", "-b:v", bitrate.as_str()].map(s)),
        _ => args.extend([s("-b:v"), bitrate]),
    }
    // Keyframe a cada segundo: o segmentador só corta em keyframe.
    args.extend([s("-g"), fps_s]);
    if audio_inputs > 0 {
        args.extend(["-c:a", "aac", "-b:a", "160k", "-ar", "48000"].map(s));
    }

    let wrap = cfg.buffer_secs.div_ceil(SEGMENT_SECS) + 4;
    args.extend([
        s("-f"),
        s("segment"),
        s("-segment_time"),
        SEGMENT_SECS.to_string(),
        s("-segment_format"),
        s("mpegts"),
        s("-segment_wrap"),
        wrap.to_string(),
        s("-reset_timestamps"),
        s("1"),
        buffer_dir.join("seg%03d.ts").to_string_lossy().into_owned(),
    ]);
    (args, uses_stdin)
}

fn video_filter(pipeline: &VideoPipeline, cfg: &Config) -> Option<String> {
    if pipeline.direct_hw {
        return None;
    }
    let mut chain: Vec<String> = Vec::new();
    if pipeline.capture == Capture::Ddagrab {
        chain.push(s("hwdownload"));
        chain.push(s("format=bgra"));
    }
    if let Some(h) = cfg.max_height() {
        chain.push(format!("scale=-2:'min(ih,{h})'"));
    }
    chain.push(match pipeline.encoder.as_str() {
        "libx264" => s("format=yuv420p"),
        "h264_vaapi" => s("format=nv12,hwupload"),
        _ => s("format=nv12"),
    });
    Some(chain.join(","))
}

/// Entrada do x11grab e tamanho do monitor escolhido (via xrandr).
fn x11_target(monitor: u32) -> (String, Option<String>) {
    let mut display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    if !display.contains('.') {
        display.push_str(".0");
    }
    let geometry = command("xrandr")
        .arg("--listactivemonitors")
        .output()
        .ok()
        .and_then(|o| parse_xrandr_monitor(&String::from_utf8_lossy(&o.stdout), monitor));
    match geometry {
        Some((w, h, x, y)) => (format!("{display}+{x},{y}"), Some(format!("{w}x{h}"))),
        None => (display, None),
    }
}

/// Linha do xrandr: ` 0: +*DP-1 2560/597x1440/336+0+0  DP-1`
fn parse_xrandr_monitor(out: &str, monitor: u32) -> Option<(u32, u32, i32, i32)> {
    let line = out.lines().find(|l| l.trim_start().starts_with(&format!("{monitor}:")))?;
    let geo = line.split_whitespace().nth(2)?;
    let (w, rest) = geo.split_once('/')?;
    let (_, rest) = rest.split_once('x')?;
    let (h, rest) = rest.split_once('/')?;
    let mut offs = rest.splitn(3, '+').skip(1);
    let x = offs.next()?.parse().ok()?;
    let y = offs.next()?.parse().ok()?;
    Some((w.parse().ok()?, h.parse().ok()?, x, y))
}

#[cfg(target_os = "linux")]
fn pulse_sources(cfg: &Config) -> Vec<String> {
    let pactl = |arg: &str| {
        command("pactl")
            .arg(arg)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mut sources = Vec::new();
    if cfg.system_audio {
        sources.push(
            pactl("get-default-sink")
                .map(|sink| format!("{sink}.monitor"))
                .unwrap_or_else(|| "@DEFAULT_MONITOR@".into()),
        );
    }
    if cfg.mic {
        sources.push(pactl("get-default-source").unwrap_or_else(|| "default".into()));
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::parse_xrandr_monitor;

    #[test]
    fn xrandr() {
        let out = "Monitors: 2\n 0: +*DP-1 2560/597x1440/336+0+0  DP-1\n 1: +HDMI-1 1920/527x1080/296+2560+180  HDMI-1\n";
        assert_eq!(parse_xrandr_monitor(out, 0), Some((2560, 1440, 0, 0)));
        assert_eq!(parse_xrandr_monitor(out, 1), Some((1920, 1080, 2560, 180)));
        assert_eq!(parse_xrandr_monitor(out, 2), None);
    }
}
