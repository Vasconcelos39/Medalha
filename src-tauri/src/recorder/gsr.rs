//! Backend Linux com gpu-screen-recorder: replay buffer nativo na GPU, funciona
//! em X11 e Wayland (KMS ou xdg-desktop-portal). Salvar = SIGUSR1.

use crate::config::Config;
use crate::util::{command, is_wayland, join_tail, tail_reader, which};
use std::collections::VecDeque;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FLATPAK_ID: &str = "com.dec05eba.gpu_screen_recorder";
/// Casa o binário dentro do sandbox, mas não o `flatpak run --command=...`.
const PKILL_PATTERN: &str = "(^|/)gpu-screen-recorder( |$)";

#[derive(Clone, Debug)]
pub struct GsrCommand {
    program: PathBuf,
    prefix: Vec<String>,
    flatpak: bool,
}

impl GsrCommand {
    pub fn describe(&self) -> String {
        if self.flatpak {
            format!("flatpak {FLATPAK_ID}")
        } else {
            self.program.display().to_string()
        }
    }
}

pub fn find() -> Option<GsrCommand> {
    if let Some(program) = which("gpu-screen-recorder") {
        return Some(GsrCommand { program, prefix: vec![], flatpak: false });
    }
    let flatpak = which("flatpak")?;
    let installed = command(&flatpak)
        .args(["info", FLATPAK_ID])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    installed.then(|| GsrCommand {
        program: flatpak,
        prefix: vec!["run".into(), "--command=gpu-screen-recorder".into(), FLATPAK_ID.into()],
        flatpak: true,
    })
}

pub type OnSaved = Arc<dyn Fn(PathBuf) + Send + Sync>;

pub struct GsrSession {
    child: Child,
    flatpak: bool,
    stderr: Arc<Mutex<VecDeque<String>>>,
    pub target: String,
}

impl GsrSession {
    pub fn start(gsr: &GsrCommand, cfg: &Config, out_dir: &Path, on_saved: OnSaved) -> Result<Self, String> {
        std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let target = match cfg.gsr_capture.as_str() {
            "auto" | "" => if is_wayland() { "portal" } else { "screen" },
            other => other,
        }
        .to_string();
        let quality = match cfg.quality.as_str() {
            "medium" => "medium",
            "ultra" => "ultra",
            _ => "very_high",
        };
        let fps = cfg.fps.to_string();
        let secs = cfg.buffer_secs.to_string();
        let out = out_dir.to_string_lossy().into_owned();
        let mut args: Vec<String> = gsr.prefix.clone();
        args.extend(
            [
                "-w", target.as_str(),
                "-f", fps.as_str(),
                "-r", secs.as_str(),
                "-c", "mp4",
                "-q", quality,
                "-v", "no",
                "-o", out.as_str(),
            ]
            .map(String::from),
        );
        if target == "portal" {
            args.extend(["-restore-portal-session", "yes"].map(String::from));
        }
        let audio = match (cfg.system_audio, cfg.mic) {
            (true, true) => Some("default_output|default_input"),
            (true, false) => Some("default_output"),
            (false, true) => Some("default_input"),
            (false, false) => None,
        };
        if let Some(a) = audio {
            args.extend(["-a", a].map(String::from));
        }

        let mut child = command(&gsr.program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("não consegui iniciar o gpu-screen-recorder: {e}"))?;
        let stderr = tail_reader(child.stderr.take().expect("stderr"), 12);

        // O gsr imprime o caminho de cada replay salvo no stdout.
        let stdout = child.stdout.take().expect("stdout");
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                let line = line.trim();
                if line.ends_with(".mp4") && Path::new(line).is_file() {
                    on_saved(PathBuf::from(line));
                }
            }
        });

        Ok(Self { child, flatpak: gsr.flatpak, stderr, target })
    }

    pub fn save(&self) -> Result<(), String> {
        self.signal(libc::SIGUSR1, "USR1")
    }

    pub fn poll_exit(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let tail = join_tail(&self.stderr);
                Some(if tail.is_empty() { format!("gpu-screen-recorder saiu ({status})") } else { tail })
            }
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    pub fn stop(mut self) {
        let _ = self.signal(libc::SIGINT, "INT");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn signal(&self, sig: libc::c_int, name: &str) -> Result<(), String> {
        if self.flatpak {
            // O PID do `flatpak run` não repassa sinais ao processo do sandbox.
            let ok = command("pkill")
                .args([format!("-{name}"), "-f".into(), PKILL_PATTERN.into()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            return if ok { Ok(()) } else { Err("gpu-screen-recorder não encontrado".into()) };
        }
        let rc = unsafe { libc::kill(self.child.id() as libc::pid_t, sig) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}
