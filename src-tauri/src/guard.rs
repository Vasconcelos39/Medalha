//! Percebe quando o Medalha acabou de repente com o buffer ligado (o computador
//! travou, faltou luz, o app foi morto) para não religar a gravação sozinho na
//! próxima vez: se foi a captura que derrubou o sistema, ligar de novo derruba de novo.
//!
//! Um arquivo marca o buffer ligado e é apagado no stop. Desligar o computador ou
//! sair da sessão manda SIGTERM, que também apaga a marca; ela só sobra depois de
//! um fim abrupto. No Windows o desligamento não passa pelo stop, então lá a marca
//! é ignorada.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static MARKER: OnceLock<PathBuf> = OnceLock::new();

/// Prepara a marca e devolve quando o buffer tinha sido ligado, se a última
/// execução terminou com ele gravando.
pub fn init(cache_dir: &Path) -> Option<String> {
    let path = cache_dir.join("buffer-ligado");
    let previous = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    let _ = MARKER.set(path);
    #[cfg(target_os = "linux")]
    signals::install();
    previous.filter(|_| cfg!(target_os = "linux"))
}

pub fn arm() {
    if let Some(path) = MARKER.get() {
        let _ = std::fs::write(path, chrono::Local::now().format("%d/%m %H:%M").to_string());
    }
}

pub fn disarm() {
    if let Some(path) = MARKER.get() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(target_os = "linux")]
mod signals {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::OnceLock;

    static MARKER_C: OnceLock<CString> = OnceLock::new();

    /// Só `unlink` e `raise` aqui dentro: são seguras num tratador de sinal.
    extern "C" fn on_signal(sig: libc::c_int) {
        if let Some(path) = MARKER_C.get() {
            unsafe { libc::unlink(path.as_ptr()) };
        }
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }

    pub fn install() {
        let Some(path) = super::MARKER.get() else { return };
        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return };
        let _ = MARKER_C.set(c);
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            unsafe { libc::signal(sig, handler) };
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn marker_survives_only_abrupt_exits() {
        let dir = std::env::temp_dir().join(format!("medalha-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(super::init(&dir), None);
        super::arm();
        assert!(dir.join("buffer-ligado").exists());
        super::disarm();
        assert!(!dir.join("buffer-ligado").exists());
        super::arm();
        // Simula a próxima abertura depois de um travamento.
        let previous = std::fs::read_to_string(dir.join("buffer-ligado")).ok();
        assert!(previous.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
