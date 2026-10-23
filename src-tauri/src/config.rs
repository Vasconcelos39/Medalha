use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// "auto" | "ffmpeg" | "gsr"
    pub backend: String,
    pub clips_dir: PathBuf,
    /// Segundos guardados no replay buffer.
    pub buffer_secs: u32,
    pub fps: u32,
    /// "medium" | "high" | "ultra"
    pub quality: String,
    /// "native" | "1440" | "1080" | "720"
    pub resolution: String,
    /// "auto" ou um encoder do ffmpeg (h264_nvenc, h264_amf, h264_qsv, h264_vaapi, libx264)
    pub encoder: String,
    pub monitor: u32,
    pub system_audio: bool,
    pub mic: bool,
    pub hotkey: String,
    pub start_on_launch: bool,
    pub close_to_tray: bool,
    pub notify_on_save: bool,
    /// Caminho manual do ffmpeg (vazio = procurar sozinho).
    pub ffmpeg_path: String,
    /// gpu-screen-recorder: "auto" | "screen" | "portal" | "focused"
    pub gsr_capture: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            clips_dir: PathBuf::new(),
            buffer_secs: 30,
            fps: 60,
            quality: "high".into(),
            resolution: "native".into(),
            encoder: "auto".into(),
            monitor: 0,
            system_audio: true,
            mic: false,
            hotkey: "F8".into(),
            start_on_launch: true,
            close_to_tray: true,
            notify_on_save: true,
            ffmpeg_path: String::new(),
            gsr_capture: "auto".into(),
        }
    }
}

impl Config {
    pub fn load(path: &Path, default_clips_dir: PathBuf) -> Self {
        let mut cfg: Config = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if cfg.clips_dir.as_os_str().is_empty() {
            cfg.clips_dir = default_clips_dir;
        }
        cfg.sanitize();
        cfg
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn sanitize(&mut self) {
        self.buffer_secs = self.buffer_secs.clamp(5, 600);
        self.fps = self.fps.clamp(10, 240);
    }

    /// Mudou algo que exige reiniciar o buffer?
    pub fn recording_changed(&self, other: &Config) -> bool {
        self.backend != other.backend
            || self.buffer_secs != other.buffer_secs
            || self.fps != other.fps
            || self.quality != other.quality
            || self.resolution != other.resolution
            || self.encoder != other.encoder
            || self.monitor != other.monitor
            || self.system_audio != other.system_audio
            || self.mic != other.mic
            || self.ffmpeg_path != other.ffmpeg_path
            || self.gsr_capture != other.gsr_capture
    }

    /// Bitrate de vídeo em kbit/s para o ffmpeg.
    pub fn video_kbps(&self) -> u32 {
        let base = match self.quality.as_str() {
            "medium" => 8_000,
            "ultra" => 30_000,
            _ => 15_000,
        };
        // 30 fps precisa de menos bits que 60.
        let scale = (self.fps as f32 / 60.0).clamp(0.6, 1.5);
        (base as f32 * scale) as u32
    }

    pub fn max_height(&self) -> Option<u32> {
        self.resolution.parse().ok()
    }
}
