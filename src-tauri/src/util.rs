use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `Command` que não abre janela de console no Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Procura um executável no PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .chain(std::iter::once(String::new()))
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Roda um comando e mata o processo se passar do tempo limite.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    };
    Some(Output {
        status,
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    })
}

/// `Jogo_2026-10-23_21-00-00.mp4` (ou `Medalha_…` sem jogo). A UI agrupa por esse prefixo.
pub fn clip_file_name(game: Option<&str>) -> String {
    let prefix = game.filter(|g| !g.is_empty()).unwrap_or("Medalha");
    format!("{prefix}_{}.mp4", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"))
}

/// String UTF-16 terminada em zero, para a API do Windows.
#[cfg(windows)]
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// `dir/name`, ou `dir/name (2)` etc. se já existir.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}).{ext}")))
        .find(|p| !p.exists())
        .expect("sequência infinita")
}

#[cfg_attr(windows, allow(dead_code))]
/// Move um arquivo, copiando quando a origem está em outro sistema de arquivos.
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}

pub fn session_kind() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if is_wayland() {
        "wayland"
    } else {
        "x11"
    }
}

/// Últimas linhas de um stream de texto, para mensagens de erro.
pub fn tail_reader(
    reader: impl Read + Send + 'static,
    keep: usize,
) -> std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>> {
    use std::io::BufRead;
    let tail = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let t = tail.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(reader).lines().map_while(Result::ok) {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            let mut t = t.lock().unwrap();
            t.push_back(line);
            while t.len() > keep {
                t.pop_front();
            }
        }
    });
    tail
}

pub fn join_tail(tail: &std::sync::Mutex<std::collections::VecDeque<String>>) -> String {
    tail.lock()
        .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}
