//! Ciclo de vida do replay buffer: escolhe o backend, mantém a sessão viva e salva clipes.

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
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "linux")]
const GSR_HINT: &str = "No Wayland a captura precisa do gpu-screen-recorder, e ele não está instalado \
    (nem no sistema nem como Flatpak). Clique em \"Instalar gpu-screen-recorder\" para baixá-lo do Flathub.";

/// Tentativas de religar a captura quando ela cai sozinha (troca de resolução,
/// UAC, tela bloqueada…). Zera depois que a sessão fica de pé por `STABLE_AFTER`.
const MAX_RETRIES: u32 = 5;
const STABLE_AFTER: Duration = Duration::from_secs(30);

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

    /// Aviso quando a captura não acompanha o tempo real (só dá para medir no ffmpeg).
    fn performance_warning(&self) -> Option<String> {
        match self {
            Session::Ffmpeg(s) => s.saver.slowdown().filter(|r| *r > SLOW_RATIO).map(|_| SLOW_WARNING.to_string()),
            #[cfg(target_os = "linux")]
            Session::Gsr(_) => None,
        }
    }
}

const SLOW_RATIO: f64 = 1.3;
const SLOW_WARNING: &str = "A gravação não está acompanhando o tempo real e os clipes podem sair travados. \
    Tente 30 fps ou uma resolução menor nas configurações (num notebook, ligar o carregador também ajuda).";

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
    /// Correção que a UI pode oferecer para o erro atual: "install-gsr".
    pub action: Option<String>,
    /// Problema com o buffer ligado, por exemplo a máquina não dando conta do fps.
    pub warning: Option<String>,
}

struct Inner {
    session: Option<Session>,
    status: Status,
    /// Incrementa a cada start/stop; threads antigas se aposentam ao ver outro valor.
    generation: u64,
    retries: u32,
    pipeline: Option<(String, media::VideoPipeline)>,
}

pub struct Recorder {
    inner: Mutex<Inner>,
    /// Criar e encerrar sessões acontece uma de cada vez. Sem isso, um start
    /// cancelado podia apagar a pasta do buffer que o start seguinte já usava.
    lifecycle: Mutex<()>,
    saving: AtomicBool,
    /// Jogo em foco quando o usuário pediu o replay; o gsr só avisa depois que salvou.
    #[cfg(target_os = "linux")]
    pending_game: Mutex<Option<String>>,
}

pub fn emit_error(app: &AppHandle, message: impl Into<String>) {
    let message = message.into();
    let _ = app.emit("clip-error", message.clone());
    // Quem aperta o atalho está no jogo e não vê a janela do Medalha.
    let app = app.clone();
    std::thread::spawn(move || clips::alert(&app, "Clipe não salvo", &message, "error"));
}

#[cfg(target_os = "linux")]
pub use gsr::install as install_gsr;

#[cfg(not(target_os = "linux"))]
pub fn install_gsr() -> Result<(), String> {
    Err("O gpu-screen-recorder só existe no Linux.".into())
}

#[cfg(target_os = "linux")]
fn suggested_action(error: &str) -> Option<String> {
    (error == GSR_HINT).then(|| "install-gsr".into())
}

#[cfg(not(target_os = "linux"))]
fn suggested_action(_error: &str) -> Option<String> {
    None
}

/// Nome do jogo para o arquivo, se a opção estiver ligada.
fn focused_game(app: &AppHandle) -> Option<String> {
    let enabled = app.state::<AppState>().config.lock().unwrap().game_in_name;
    enabled.then(game::focused).flatten()
}

fn recorder(app: &AppHandle) -> &Recorder {
    &app.state::<AppState>().inner().recorder
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
                    action: None,
                    warning: None,
                },
                generation: 0,
                retries: 0,
                pipeline: None,
            }),
            lifecycle: Mutex::new(()),
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
            inner.retries = 0;
            Self::mark_starting(&mut inner)
        };
        self.emit(app);
        Self::spawn_start(app, generation, Duration::ZERO);
    }

    fn mark_starting(inner: &mut Inner) -> u64 {
        inner.generation += 1;
        inner.status.state = "starting".into();
        inner.status.error = None;
        inner.status.detail = None;
        inner.status.action = None;
        inner.status.warning = None;
        inner.status.since = None;
        inner.generation
    }

    fn spawn_start(app: &AppHandle, generation: u64, delay: Duration) {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let rec = recorder(&app);
            let _lifecycle = rec.lifecycle.lock().unwrap();
            if rec.inner.lock().unwrap().generation != generation {
                return;
            }
            let state = app.state::<AppState>();
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
                    inner.status.action = suggested_action(&e);
                    inner.status.error = Some(e);
                }
            }
            let recording = inner.session.is_some();
            drop(inner);
            rec.emit(&app);
            if recording {
                rec.watch_health(&app, generation);
            }
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
                    detail.push_str(", e no Wayland o x11grab só enxerga janelas XWayland");
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
                // Pasta escondida dentro da de clipes: o Flatpak do gsr enxerga os Vídeos,
                // e mover dali para o lugar final é só um rename no mesmo disco.
                let tmp_dir = cfg.clips_dir.join(".medalha-replay");
                let session = gsr::GsrSession::start(&cmd, cfg, &tmp_dir, on_saved)?;
                let detail = format!("{}, captura {}", cmd.describe(), session.target);
                Ok((Session::Gsr(session), "gpu-screen-recorder".into(), detail))
            }
        }
    }

    /// Acompanha a sessão; se o processo de captura morrer sozinho, tenta religar.
    fn watch_health(&self, app: &AppHandle, generation: u64) {
        let app = app.clone();
        let started = Instant::now();
        let mut tick = 0u64;
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            tick += 1;
            let rec = recorder(&app);
            let mut inner = rec.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            if started.elapsed() >= STABLE_AFTER {
                inner.retries = 0;
            }
            let Some(session) = inner.session.as_mut() else { return };
            let Some(err) = session.poll_exit() else {
                if tick % 10 == 0 {
                    let warning = session.performance_warning();
                    if warning != inner.status.warning {
                        inner.status.warning = warning;
                        drop(inner);
                        rec.emit(&app);
                    }
                }
                continue;
            };

            let session = inner.session.take();
            let retry = inner.retries < MAX_RETRIES;
            let next = if retry {
                inner.retries += 1;
                let attempt = inner.retries;
                Some((Self::mark_starting(&mut inner), Duration::from_secs(2 * attempt as u64)))
            } else {
                inner.status.state = "error".into();
                inner.status.error = Some(err);
                inner.status.since = None;
                None
            };
            drop(inner);
            if let Some(s) = session {
                let _lifecycle = rec.lifecycle.lock().unwrap();
                s.stop();
            }
            rec.emit(&app);
            if let Some((generation, delay)) = next {
                Self::spawn_start(&app, generation, delay);
            }
            return;
        });
    }

    /// Pode bloquear até o processo de captura encerrar; chame fora da thread principal.
    pub fn stop(&self, app: &AppHandle) {
        let session = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.retries = 0;
            inner.status.state = "idle".into();
            inner.status.error = None;
            inner.status.action = None;
            inner.status.warning = None;
            inner.status.since = None;
            inner.session.take()
        };
        // Sem sessão não há o que esperar: um start em andamento se cancela sozinho
        // ao ver a geração nova (e o app não trava ao sair no meio de um probe).
        if let Some(s) = session {
            let _lifecycle = self.lifecycle.lock().unwrap();
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

    /// Reaplica as configurações. Também tenta de novo depois de um erro.
    pub fn restart(&self, app: &AppHandle) {
        let should = self.is_active() || self.inner.lock().unwrap().status.state == "error";
        if should {
            self.stop(app);
            self.start(app);
        }
    }

    /// Volta na hora; o trabalho roda numa thread própria porque o atalho global e
    /// os comandos chegam pela thread da interface, e descobrir o jogo leva um instante.
    pub fn save(&self, app: &AppHandle) {
        let app = app.clone();
        std::thread::spawn(move || recorder(&app).save_now(&app));
    }

    fn save_now(&self, app: &AppHandle) {
        let game = focused_game(app);
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
                let cfg = app.state::<AppState>().config.lock().unwrap().clone();
                let result = {
                    // Segura o stop até o ffmpeg terminar de ler os segmentos.
                    let _lifecycle = self.lifecycle.lock().unwrap();
                    std::fs::create_dir_all(&cfg.clips_dir)
                        .map_err(|e| e.to_string())
                        .map(|()| util::unique_path(&cfg.clips_dir, &util::clip_file_name(game.as_deref())))
                        .and_then(|out| saver.save(&out, cfg.buffer_secs).map(|()| out))
                };
                self.saving.store(false, Ordering::SeqCst);
                match result {
                    Ok(out) => clips::on_clip_saved(app, out),
                    Err(e) => emit_error(app, format!("Falha ao salvar o clipe: {e}")),
                }
            }
            #[cfg(target_os = "linux")]
            Some(Session::Gsr(s)) => {
                *self.pending_game.lock().unwrap() = game;
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
