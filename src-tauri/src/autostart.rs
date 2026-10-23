//! "Iniciar com o sistema": valor `Run` no registro (Windows) ou `.desktop` em
//! `~/.config/autostart` (Linux). Sempre inicia com `--hidden`, direto na bandeja.

pub fn set(enabled: bool) -> Result<(), String> {
    imp::set(enabled).map_err(|e| format!("Não consegui configurar a inicialização com o sistema: {e}"))
}

#[cfg(windows)]
mod imp {
    use crate::util::wide;
    use std::ffi::c_void;
    use std::io;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "Medalha";
    /// `HKEY_CURRENT_USER` é `(HKEY)(LONG)0x80000001`, com extensão de sinal.
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const REG_SZ: u32 = 1;
    const ERROR_FILE_NOT_FOUND: i32 = 2;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegSetKeyValueW(key: isize, sub_key: *const u16, value: *const u16, kind: u32, data: *const c_void, len: u32) -> i32;
        fn RegDeleteKeyValueW(key: isize, sub_key: *const u16, value: *const u16) -> i32;
    }

    pub fn set(enabled: bool) -> io::Result<()> {
        let key = wide(RUN_KEY);
        let name = wide(VALUE_NAME);
        let rc = if enabled {
            let exe = std::env::current_exe()?;
            let data = wide(format!("\"{}\" --hidden", exe.display()));
            let len = (data.len() * std::mem::size_of::<u16>()) as u32;
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), len) }
        } else {
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => 0,
                rc => rc,
            }
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(rc))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::PathBuf;

    pub fn set(enabled: bool) -> io::Result<()> {
        let file = desktop_file()?;
        if !enabled {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        // No AppImage o executável roda de um mount temporário; o caminho estável vem em $APPIMAGE.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(p) => PathBuf::from(p),
            None => std::env::current_exe()?,
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, desktop_entry(&exe.to_string_lossy()))
    }

    fn desktop_file() -> io::Result<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME não definido"))?;
        Ok(config.join("autostart").join("medalha.desktop"))
    }

    pub(super) fn desktop_entry(exe: &str) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Medalha\nComment=Replay buffer do Medalha\n\
             Exec={} --hidden\nIcon=medalha\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec_quote(exe)
        )
    }

    /// Aspas da especificação Desktop Entry: `"`, `` ` ``, `$` e `\` escapados; `\` vira
    /// `\\\\` porque o escape de string é aplicado antes; `%` vira `%%`.
    fn exec_quote(arg: &str) -> String {
        let mut out = String::from("\"");
        for c in arg.chars() {
            match c {
                '"' | '`' | '$' => {
                    out.push('\\');
                    out.push(c);
                }
                '\\' => out.push_str(r"\\\\"),
                '%' => out.push_str("%%"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn quotes_exec() {
            assert_eq!(super::exec_quote("/usr/bin/medalha"), "\"/usr/bin/medalha\"");
            assert_eq!(super::exec_quote("/home/a b/$x%.AppImage"), "\"/home/a b/\\$x%%.AppImage\"");
            assert!(super::desktop_entry("/usr/bin/medalha").contains("\nExec=\"/usr/bin/medalha\" --hidden\n"));
        }
    }
}
