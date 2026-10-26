//! Descobre o jogo (a janela em foco) na hora de salvar, para nomear o clipe.

/// Processos que não são jogo: o próprio Medalha, a área de trabalho, o menu iniciar…
const IGNORED: &[&str] = &[
    "medalha",
    "explorer",
    "searchhost",
    "searchapp",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "lockapp",
    "textinputhost",
    "applicationframehost",
    "gnome-shell",
    "plasmashell",
    "xfdesktop",
    "xfce4-panel",
];

/// Apps comuns que não são jogo: nome curto no lugar do título da aba ou conversa.
const KNOWN_APPS: &[(&str, &str)] = &[
    ("chrome", "Chrome"),
    ("msedge", "Edge"),
    ("firefox", "Firefox"),
    ("brave", "Brave"),
    ("opera", "Opera"),
    ("discord", "Discord"),
    ("steam", "Steam"),
    ("steamwebhelper", "Steam"),
    ("spotify", "Spotify"),
    ("obs64", "OBS"),
    ("obs", "OBS"),
    ("code", "VS Code"),
    ("windowsterminal", "Terminal"),
    ("openconsole", "Terminal"),
    ("conhost", "Terminal"),
    ("cmd", "Terminal"),
    ("powershell", "Terminal"),
    ("pwsh", "Terminal"),
    ("konsole", "Terminal"),
    ("gnome-terminal-", "Terminal"),
    ("kitty", "Terminal"),
    ("alacritty", "Terminal"),
    ("wezterm-gui", "Terminal"),
    ("foot", "Terminal"),
    ("ptyxis", "Terminal"),
];

const MAX_CHARS: usize = 40;

/// Nome do jogo em foco, pronto para usar no nome do arquivo.
pub fn focused() -> Option<String> {
    let (title, exe) = imp::focused()?;
    pick_name(&title, exe.as_deref())
}

fn pick_name(title: &str, exe: Option<&str>) -> Option<String> {
    let exe = exe.map(str::to_lowercase);
    if let Some(exe) = exe.as_deref() {
        if IGNORED.contains(&exe) {
            return None;
        }
        if let Some((_, app)) = KNOWN_APPS.iter().find(|(e, _)| *e == exe) {
            return Some(app.to_string());
        }
    }
    clean(title).or_else(|| clean(exe.as_deref()?))
}

/// Tira caracteres proibidos em nomes de arquivo, ™/®, espaços repetidos e corta o tamanho.
fn clean(raw: &str) -> Option<String> {
    let mut out = String::new();
    for c in raw.chars() {
        if matches!(c, '™' | '®' | '©') {
            continue;
        }
        if c.is_whitespace() || c.is_control() || r#"<>:"/\|?*"#.contains(c) {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    let out: String = out.chars().take(MAX_CHARS).collect();
    let out = out.trim().trim_end_matches('.').trim_end();
    (!out.is_empty()).then(|| out.to_string())
}

/// `_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4a00007` → `0x4a00007` (0x0 = nenhuma).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_active_window(out: &str) -> Option<String> {
    let id = out.split('#').nth(1)?.split(',').next()?.trim();
    (id.starts_with("0x") && id != "0x0").then(|| id.to_string())
}

/// Valor bruto de uma propriedade na saída do xprop: `NOME(TIPO) = valor`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn xprop_value<'a>(out: &'a str, name: &str) -> Option<&'a str> {
    out.lines().find_map(|line| {
        let rest = line.strip_prefix(name)?.strip_prefix('(')?;
        Some(rest.split_once(" = ")?.1.trim())
    })
}

/// Propriedade de texto: `_NET_WM_NAME(UTF8_STRING) = "Counter-Strike 2"`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn xprop_string(out: &str, name: &str) -> Option<String> {
    let quoted = xprop_value(out, name)?.strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        text.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
    }
    Some(text)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;

    type Handle = *mut c_void;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> Handle;
        fn GetWindowTextW(hwnd: Handle, text: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    pub fn focused() -> Option<(String, Option<String>)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..len]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            Some((title, process_name(pid)))
        }
    }

    unsafe fn process_name(pid: u32) -> Option<String> {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) != 0;
        CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use crate::util::{command, run_with_timeout};
    use serde_json::Value;
    use std::time::Duration;

    pub fn focused() -> Option<(String, Option<String>)> {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return hyprland();
        }
        if std::env::var_os("SWAYSOCK").is_some() {
            return sway();
        }
        // X11, ou janelas XWayland (a maioria dos jogos via Proton/Wine) no GNOME/KDE.
        x11()
    }

    fn run(program: &str, args: &[&str]) -> Option<String> {
        let mut cmd = command(program);
        cmd.args(args).env("LC_ALL", "C.UTF-8");
        let out = run_with_timeout(cmd, Duration::from_secs(2))?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn comm(pid: u64) -> Option<String> {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
    }

    fn hyprland() -> Option<(String, Option<String>)> {
        let window: Value = serde_json::from_str(&run("hyprctl", &["activewindow", "-j"])?).ok()?;
        let title = window["title"].as_str()?.to_string();
        Some((title, window["pid"].as_u64().and_then(comm)))
    }

    fn sway() -> Option<(String, Option<String>)> {
        let tree: Value = serde_json::from_str(&run("swaymsg", &["-t", "get_tree"])?).ok()?;
        let node = find_focused(&tree)?;
        let title = node["name"].as_str().unwrap_or_default().to_string();
        Some((title, node["pid"].as_u64().and_then(comm)))
    }

    /// Só janelas têm pid; um workspace vazio em foco não conta.
    fn find_focused(node: &Value) -> Option<&Value> {
        if node["focused"] == true && node["pid"].is_u64() {
            return Some(node);
        }
        ["nodes", "floating_nodes"]
            .iter()
            .filter_map(|k| node[*k].as_array())
            .flatten()
            .find_map(find_focused)
    }

    fn x11() -> Option<(String, Option<String>)> {
        let id = super::parse_active_window(&run("xprop", &["-root", "_NET_ACTIVE_WINDOW"])?)?;
        let props = run("xprop", &["-id", &id, "_NET_WM_NAME", "WM_NAME", "_NET_WM_PID"])?;
        let title = super::xprop_string(&props, "_NET_WM_NAME")
            .or_else(|| super::xprop_string(&props, "WM_NAME"))
            .unwrap_or_default();
        let exe = super::xprop_value(&props, "_NET_WM_PID")
            .and_then(|p| p.parse().ok())
            .and_then(comm);
        Some((title, exe))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn focused() -> Option<(String, Option<String>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_titles() {
        assert_eq!(clean("VALORANT  ").as_deref(), Some("VALORANT"));
        assert_eq!(clean("Counter-Strike™ 2").as_deref(), Some("Counter-Strike 2"));
        assert_eq!(clean("a/b:c?").as_deref(), Some("a b c"));
        assert_eq!(clean("  ...  "), None);
        assert_eq!(clean(&"x".repeat(80)).map(|s| s.len()), Some(MAX_CHARS));
    }

    #[test]
    fn picks_names() {
        assert_eq!(pick_name("Medalha", Some("medalha")), None);
        assert_eq!(pick_name("", Some("Explorer")), None);
        assert_eq!(pick_name("Vídeo - YouTube", Some("chrome")).as_deref(), Some("Chrome"));
        assert_eq!(pick_name("Elden Ring", Some("eldenring")).as_deref(), Some("Elden Ring"));
        assert_eq!(pick_name("", Some("cs2")).as_deref(), Some("cs2"));
        assert_eq!(pick_name("", None), None);
    }

    #[test]
    fn parses_xprop() {
        assert_eq!(
            parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4a00007\n").as_deref(),
            Some("0x4a00007")
        );
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n"), None);
        let props = "_NET_WM_NAME(UTF8_STRING) = \"Say \\\"hi\\\" 2\"\nWM_NAME(STRING) = \"old\"\n_NET_WM_PID(CARDINAL) = 4242\n";
        assert_eq!(xprop_string(props, "_NET_WM_NAME").as_deref(), Some("Say \"hi\" 2"));
        assert_eq!(xprop_string(props, "WM_NAME").as_deref(), Some("old"));
        assert_eq!(xprop_value(props, "_NET_WM_PID"), Some("4242"));
        assert_eq!(xprop_string("_NET_WM_NAME:  not found.\n", "_NET_WM_NAME"), None);
    }
}
