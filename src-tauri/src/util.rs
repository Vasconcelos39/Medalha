use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `Command` que não abre janela de console no Windows e, dentro de um AppImage,
/// não repassa o ambiente das bibliotecas empacotadas para os programas do sistema.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(target_os = "linux")]
    if let Some(appdir) = std::env::var_os("APPDIR") {
        clean_appimage_env(&mut cmd, Path::new(&appdir));
    }
    cmd
}

/// Variáveis que o AppRun e os hooks gtk/gstreamer do AppImage exportam só para o
/// próprio Medalha. Herdadas, fazem flatpak, ffmpeg, pactl etc. carregarem módulos
/// de dentro do AppImage em vez dos do sistema.
#[cfg(target_os = "linux")]
const APPIMAGE_VARS: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "GDK_BACKEND",
    "GDK_PIXBUF_MODULE_FILE",
    "GDK_PIXBUF_MODULEDIR",
    "GIO_EXTRA_MODULES",
    "GIO_MODULE_DIR",
    "GSETTINGS_SCHEMA_DIR",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GTK_PATH",
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PTP_HELPER",
    "GST_PTP_HELPER_1_0",
    "GST_REGISTRY_REUSE_PLUGIN_SCANNER",
    "PERLLIB",
    "PYTHONHOME",
    "PYTHONPATH",
    "QT_PLUGIN_PATH",
];

#[cfg(target_os = "linux")]
fn clean_appimage_env(cmd: &mut Command, appdir: &Path) {
    for var in APPIMAGE_VARS {
        cmd.env_remove(var);
    }
    for var in ["PATH", "XDG_DATA_DIRS"] {
        if let Some(value) = std::env::var_os(var) {
            cmd.env(var, without_dir(&value, appdir));
        }
    }
}

/// Lista no formato do PATH sem as entradas dentro de `dir`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn without_dir(list: &OsStr, dir: &Path) -> std::ffi::OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(list).filter(|p| !p.starts_with(dir)).collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// Como `which`, mas também olha pastas que o PATH de apps abertos pelo menu
/// costuma não ter (~/.local/bin, Homebrew do Bazzite/Universal Blue).
pub fn find_program(name: &str) -> Option<PathBuf> {
    which(name).or_else(|| extra_bin_dirs().into_iter().map(|d| d.join(name)).find(|p| p.is_file()))
}

#[cfg(target_os = "linux")]
fn extra_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.insert(0, PathBuf::from(home).join(".local/bin"));
    }
    dirs
}

#[cfg(not(target_os = "linux"))]
fn extra_bin_dirs() -> Vec<PathBuf> {
    Vec::new()
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

/// Arquivo temporário ao lado de `out`. A extensão `.part` fica fora da lista de
/// vídeos, então a aba Clipes não mostra o arquivo enquanto o ffmpeg escreve.
pub fn partial_path(out: &Path) -> PathBuf {
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    out.with_file_name(format!(".{name}.part"))
}

/// No Windows, põe o processo num job que o sistema encerra quando o Medalha fecha,
/// mesmo se ele travar ou for morto pelo Gerenciador de Tarefas.
#[cfg(windows)]
pub fn kill_with_parent(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;

    // O handle do job nunca é fechado: ele fecha sozinho quando o processo termina.
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe { job::create() });
    if job != 0 {
        unsafe { job::AssignProcessToJobObject(job as job::Handle, child.as_raw_handle()) };
    }
}

#[cfg(not(windows))]
pub fn kill_with_parent(_child: &std::process::Child) {}

#[cfg(windows)]
mod job {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const EXTENDED_LIMIT_INFORMATION: i32 = 9;

    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
        io_counters: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, len: u32) -> i32;
        pub fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    }

    pub unsafe fn create() -> usize {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let limits = ExtendedLimits { limit_flags: KILL_ON_JOB_CLOSE, ..Default::default() };
        let len = std::mem::size_of::<ExtendedLimits>() as u32;
        if SetInformationJobObject(job, EXTENDED_LIMIT_INFORMATION, (&limits as *const ExtendedLimits).cast(), len) == 0 {
            return 0;
        }
        job as usize
    }
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn drops_appimage_dirs() {
        let path = OsStr::new("/tmp/.mount_MedalhXYZ/usr/bin:/usr/bin:/tmp/.mount_MedalhXYZ/usr/sbin:/home/u/.local/bin");
        let cleaned = super::without_dir(path, Path::new("/tmp/.mount_MedalhXYZ"));
        assert_eq!(cleaned, "/usr/bin:/home/u/.local/bin");
    }
}
