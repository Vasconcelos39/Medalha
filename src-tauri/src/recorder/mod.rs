//! Gerencia o replay buffer: escolhe o backend, mantém a sessão viva e salva clipes.

#[cfg(windows)]
mod audio_win;
mod ffmpeg;
#[cfg(target_os = "linux")]
pub mod gsr;

use crate::config::Config;
use crate::{clips, game, media, util, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "linux")]
const GSR_HINT: &str = "No Wayland a captura precisa do gpu-screen-recorder: instale pelo gerenciador \
    da sua distro (ou flatpak com.dec05eba.gpu_screen_recorder) e reinicie o buffer.";

enum Session {
    Ffmpeg(ffmpeg::FfmpegSession),
    #[cfg(target_os = "linux")]
    Gsr(gsr::GsrSession),
}

impl Session {
    fn poll_exit(&mut self) -> Option<String> {
        match self {
            Session::Ffmpeg(s) => s.poll_exit(),
            #[cfg(target_os = "linux")]
            Session::Gsr(s) => s.poll_exit(),
        }
    }

    fn stop(self) {
        match self {
            Session::Ffmpeg(s) => s.stop(),
            #[cfg(target_os = "linux")]
            Session::Gsr(s) => s.stop(),
        }
    }
}

enum Backend {
    Ffmpeg,
    #[cfg(target_os = "linux")]
    Gsr(gsr::GsrCommand),
}

#[cfg(target_os = "linux")]
fn resolve_backend(cfg: &Config) -> Result<Backend, String> {
    match cfg.backend.as_str() {
        "ffmpeg" => Ok(Backend::Ffmpeg),
        "gsr" => gsr::find().map(Backend::Gsr).ok_or_else(|| GSR_HINT.to_string()),
        _ => match gsr::find() {
            Some(cmd) => Ok(Backend::Gsr(cmd)),
            None if util::is_wayland() => Err(GSR_HINT.to_string()),
            None => Ok(Backend::Ffmpeg),
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn resolve_backend(cfg: &Config) -> Result<Backend, String> {
    match cfg.backend.as_str() {
        "gsr" => Err("O gpu-screen-recorder só existe no Linux; use ffmpeg.".into()),
        _ => Ok(Backend::Ffmpeg),
    }
}

/// Nome do backend que o modo atual usaria (para o diagnóstico).
pub fn backend_label(cfg: &Config) -> String {
    match resolve_backend(cfg) {
        Ok(Backend::Ffmpeg) => "ffmpeg".into(),
        #[cfg(target_os = "linux")]
        Ok(Backend::Gsr(cmd)) => format!("gpu-screen-recorder ({})", cmd.describe()),
        Err(e) => format!("indisponível: {e}"),
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// "idle" | "starting" | "recording" | "error"
    pub state: String,
    pub backend: Option<String>,
    pub detail: Option<String>,
    pub error: Option<String>,
    /// Início do buffer (ms desde a época).
    pub since: Option<i64>,
    pub hotkey: String,
    pub hotkey_ok: bool,
}

struct Inner {
    session: Option<Session>,
    status: Status,
    /// Incrementa a cada start/stop; threads antigas se aposentam ao ver outro valor.
    generation: u64,
    pipeline: Option<(String, media::VideoPipeline)>,
}

pub struct Recorder {
    inner: Mutex<Inner>,
    saving: AtomicBool,
    /// Jogo em foco quando o usuário pediu o replay; o gsr só avisa depois que salvou.
    #[cfg(target_os = "linux")]
    pending_game: Mutex<Option<String>>,
}

pub fn emit_error(app: &AppHandle, message: impl Into<String>) {
    let _ = app.emit("clip-error", message.into());
}

/// Nome do jogo para o arquivo, se a opção estiver ligada.
fn focused_game(app: &AppHandle) -> Option<String> {
    let enabled = app.state::<AppState>().config.lock().unwrap().game_in_name;
    enabled.then(game::focused).flatten()
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                session: None,
                status: Status {
                    state: "idle".into(),
                    backend: None,
                    detail: None,
                    error: None,
                    since: None,
                    hotkey: String::new(),
                    hotkey_ok: false,
                },
                generation: 0,
                pipeline: None,
            }),
            saving: AtomicBool::new(false),
            #[cfg(target_os = "linux")]
            pending_game: Mutex::new(None),
        }
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit("recorder-status", self.status());
    }

    pub fn set_hotkey(&self, app: &AppHandle, hotkey: &str, ok: bool) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.status.hotkey = hotkey.to_string();
            inner.status.hotkey_ok = ok;
        }
        self.emit(app);
    }

    pub fn is_active(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.session.is_some() || inner.status.state == "starting"
    }

    pub fn start(&self, app: &AppHandle) {
        let generation = {
            let mut inner = self.inner.lock().unwrap();
            if inner.session.is_some() || inner.status.state == "starting" {
                return;
            }
            inner.generation += 1;
            inner.status.state = "starting".into();
            inner.status.error = None;
            inner.status.detail = None;
            inner.generation
        };
        self.emit(app);
        let app = app.clone();
        std::thread::spawn(move || {
            let state = app.state::<AppState>();
            let rec = &state.recorder;
            let cfg = state.config.lock().unwrap().clone();
            let result = rec.create_session(&app, &state, &cfg);
            let mut inner = rec.inner.lock().unwrap();
            if inner.generation != generation {
                // Pediram stop enquanto iniciava.
                drop(inner);
                if let Ok((session, ..)) = result {
                    session.stop();
                }
                return;
            }
            match result {
                Ok((session, backend, detail)) => {
                    inner.session = Some(session);
                    inner.status.state = "recording".into();
                    inner.status.backend = Some(backend);
                    inner.status.detail = Some(detail);
                    inner.status.since = Some(chrono::Utc::now().timestamp_millis());
                }
                Err(e) => {
                    inner.status.state = "error".into();
                    inner.status.error = Some(e);
                }
            }
            drop(inner);
            rec.emit(&app);
            rec.watch_health(&app, generation);
        });
    }

    fn create_session(
        &self,
        #[allow(unused_variables)] app: &AppHandle,
        state: &AppState,
        cfg: &Config,
    ) -> Result<(Session, String, String), String> {
        match resolve_backend(cfg)? {
            Backend::Ffmpeg => {
                let ffmpeg = media::find_ffmpeg(&cfg.ffmpeg_path).ok_or(
                    "ffmpeg não encontrado. Instale-o (Windows: winget install Gyan.FFmpeg) \
                     ou indique o caminho nas configurações.",
                )?;
                let key = media::pipeline_key(&ffmpeg, cfg);
                let cached = self
                    .inner
                    .lock()
                    .unwrap()
                    .pipeline
                    .clone()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, p)| p);
                let pipeline = match cached {
                    Some(p) => p,
                    None => {
                        let p = media::resolve_pipeline(&ffmpeg, cfg)?;
                        self.inner.lock().unwrap().pipeline = Some((key, p.clone()));
                        p
                    }
                };
                let session = ffmpeg::FfmpegSession::start(&ffmpeg, &pipeline, cfg, &state.cache_dir.join("buffer"))?;
                let mut detail = pipeline.describe();
                if util::is_wayland() {
                    detail.push_str(" — no Wayland o x11grab só enxerga janelas XWayland");
                }
                Ok((Session::Ffmpeg(session), "ffmpeg".into(), detail))
            }
            #[cfg(target_os = "linux")]
            Backend::Gsr(cmd) => {
                let app = app.clone();
                let on_saved: gsr::OnSaved = std::sync::Arc::new(move |tmp: std::path::PathBuf| {
                    let state = app.state::<AppState>();
                    let dir = state.config.lock().unwrap().clips_dir.clone();
                    let _ = std::fs::create_dir_all(&dir);
                    let game = state.recorder.pending_game.lock().unwrap().take();
                    let dest = util::unique_path(&dir, &util::clip_file_name(game.as_deref()));
                    match util::move_file(&tmp, &dest) {
                        Ok(()) => clips::on_clip_saved(&app, dest),
                        Err(e) => emit_error(&app, format!("Não consegui mover o clipe: {e}")),
                    }
                });
                let session = gsr::GsrSession::start(&cmd, cfg, &state.cache_dir.join("gsr"), on_saved)?;
                let detail = format!("{} · captura {}", cmd.describe(), session.target);
                Ok((Session::Gsr(session), "gpu-screen-recorder".into(), detail))
            }
        }
    }

    fn watch_health(&self, app: &AppHandle, generation: u64) {
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = app.state::<AppState>();
            let rec = &state.recorder;
            let mut inner = rec.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            let Some(err) = inner.session.as_mut().and_then(Session::poll_exit) else {
                if inner.session.is_none() {
                    return;
                }
                continue;
            };
            let session = inner.session.take();
            inner.status.state = "error".into();
            inner.status.error = Some(err);
            inner.status.since = None;
            drop(inner);
            if let Some(s) = session {
                s.stop();
            }
            rec.emit(&app);
            return;
        });
    }

    /// Bloqueia até o processo de captura encerrar; chame fora da thread principal.
    pub fn stop(&self, app: &AppHandle) {
        let session = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.status.state = "idle".into();
            inner.status.error = None;
            inner.status.since = None;
            inner.session.take()
        };
        if let Some(s) = session {
            s.stop();
        }
        self.emit(app);
    }

    pub fn toggle(&self, app: &AppHandle) {
        if self.is_active() {
            self.stop(app);
        } else {
            self.start(app);
        }
    }

    pub fn restart(&self, app: &AppHandle) {
        if self.is_active() {
            self.stop(app);
            self.start(app);
        }
    }

    pub fn save(&self, app: &AppHandle) {
        let mut inner = self.inner.lock().unwrap();
        match inner.session.as_mut() {
            None => {
                drop(inner);
                emit_error(app, "O buffer não está ativo.");
            }
            Some(Session::Ffmpeg(s)) => {
                if self.saving.swap(true, Ordering::SeqCst) {
                    return;
                }
                let saver = s.saver.clone();
                drop(inner);
                let _ = app.emit("clip-saving", ());
                let app = app.clone();
                std::thread::spawn(move || {
                    let game = focused_game(&app);
                    let state = app.state::<AppState>();
                    let cfg = state.config.lock().unwrap().clone();
                    let result = std::fs::create_dir_all(&cfg.clips_dir)
                        .map_err(|e| e.to_string())
                        .map(|()| util::unique_path(&cfg.clips_dir, &util::clip_file_name(game.as_deref())))
                        .and_then(|out| saver.save(&out, cfg.buffer_secs).map(|()| out));
                    state.recorder.saving.store(false, Ordering::SeqCst);
                    match result {
                        Ok(out) => clips::on_clip_saved(&app, out),
                        Err(e) => emit_error(&app, format!("Falha ao salvar o clipe: {e}")),
                    }
                });
            }
            #[cfg(target_os = "linux")]
            Some(Session::Gsr(s)) => {
                *self.pending_game.lock().unwrap() = focused_game(app);
                let result = s.save();
                drop(inner);
                match result {
                    Ok(()) => {
                        let _ = app.emit("clip-saving", ());
                    }
                    Err(e) => emit_error(app, format!("Falha ao salvar o clipe: {e}")),
                }
            }
        }
    }
}
