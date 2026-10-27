//! Aviso de versão nova pelas releases do GitHub e atualização com um clique.
//!
//! - AppImage: baixa o novo ao lado do atual, troca o arquivo e reabre.
//! - .deb / .rpm: baixa o pacote e instala com `pkexec` (o sistema pede a senha).
//! - Windows: baixa o instalador NSIS e abre ele depois que o Medalha fecha.
//! - Outros jeitos de instalar: só avisa e abre a página da release.

use crate::util::command;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const LATEST: &str = "https://api.github.com/repos/Vasconcelos39/Medalha/releases/latest";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// "appimage" | "deb" | "rpm" | "installer" | "manual"
    pub method: String,
    #[serde(skip)]
    asset: Option<Asset>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: u64,
}

fn agent(app: &AppHandle) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .user_agent(&format!("Medalha/{}", app.package_info().version))
        .build()
}

fn version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim().trim_start_matches('v').split(['.', '-']);
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Jeito como esta cópia do Medalha foi instalada.
fn install_method() -> &'static str {
    if cfg!(windows) {
        return "installer";
    }
    if std::env::var_os("APPIMAGE").is_some() {
        return "appimage";
    }
    if Path::new("/var/lib/dpkg/info/medalha.list").exists() {
        return "deb";
    }
    let rpm = command("rpm").args(["-q", "medalha"]).output();
    if rpm.is_ok_and(|o| o.status.success()) {
        return "rpm";
    }
    "manual"
}

fn asset_for(method: &str, assets: &[Asset]) -> Option<Asset> {
    let suffix = match method {
        "installer" => "-setup.exe",
        "appimage" => ".AppImage",
        "deb" => ".deb",
        "rpm" => ".rpm",
        _ => return None,
    };
    assets.iter().find(|a| a.name.ends_with(suffix)).cloned()
}

/// Consulta a última release. `None` se já está na versão mais nova.
pub fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let release: Release = agent(app)
        .get(LATEST)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("não consegui consultar o GitHub: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let current = version(&app.package_info().version.to_string());
    let latest = version(&release.tag_name);
    if latest.is_none() || latest <= current {
        return Ok(None);
    }
    let mut method = install_method().to_string();
    let asset = asset_for(&method, &release.assets);
    if asset.is_none() {
        method = "manual".into();
    }
    Ok(Some(UpdateInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        notes: release.body.unwrap_or_default(),
        page: release.html_url,
        method,
        asset,
    }))
}

/// Procura atualização agora e depois a cada 6 horas, enquanto a opção estiver ligada.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        loop {
            let enabled = app.state::<crate::AppState>().config.lock().unwrap().check_updates;
            if enabled {
                if let Ok(Some(info)) = check(&app) {
                    *app.state::<crate::AppState>().update.lock().unwrap() = Some(info.clone());
                    let _ = app.emit("update-available", info);
                }
            }
            std::thread::sleep(Duration::from_secs(6 * 60 * 60));
        }
    });
}

fn download(app: &AppHandle, asset: &Asset, to: &Path) -> Result<(), String> {
    let response = agent(app)
        .get(&asset.browser_download_url)
        .call()
        .map_err(|e| format!("download falhou: {e}"))?;
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download falhou: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        let _ = app.emit("update-progress", Progress { downloaded, total: asset.size });
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if downloaded != asset.size {
        return Err(format!("download incompleto ({downloaded} de {} bytes)", asset.size));
    }
    Ok(())
}

/// Baixa e instala. Em caso de sucesso o Medalha fecha e reabre já atualizado.
pub fn install(app: &AppHandle, info: &UpdateInfo) -> Result<(), String> {
    let asset = info.asset.as_ref().ok_or("Esta instalação não se atualiza sozinha.")?;
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(&asset.name);
    download(app, asset, &file)?;
    match info.method.as_str() {
        "appimage" => replace_appimage(&file),
        "deb" => system_install(&file, &[&["apt-get", "install", "-y", "--allow-downgrades"]]),
        "rpm" => system_install(&file, &[&["dnf", "install", "-y"], &["zypper", "--non-interactive", "install"]]),
        "installer" => Ok(()),
        _ => Err("Esta instalação não se atualiza sozinha.".into()),
    }?;
    relaunch(app, &file)
}

#[cfg(unix)]
fn replace_appimage(new: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let current = std::path::PathBuf::from(std::env::var_os("APPIMAGE").ok_or("APPIMAGE não definido")?);
    // Copia para a mesma pasta e troca com rename, que é atômico; o processo atual
    // segue rodando do arquivo antigo, já aberto.
    let staged = crate::util::partial_path(&current);
    std::fs::copy(new, &staged).map_err(|e| format!("não consegui gravar ao lado do AppImage: {e}"))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    std::fs::rename(&staged, &current).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn replace_appimage(_new: &Path) -> Result<(), String> {
    Err("AppImage só existe no Linux.".into())
}

/// Instala o pacote como root via polkit, com o primeiro gerenciador que existir.
fn system_install(package: &Path, managers: &[&[&str]]) -> Result<(), String> {
    let pkexec = crate::util::find_program("pkexec").ok_or("pkexec não encontrado para instalar o pacote.")?;
    let manager = managers
        .iter()
        .find(|m| crate::util::find_program(m[0]).is_some())
        .ok_or("Nenhum gerenciador de pacotes conhecido.")?;
    let out = command(pkexec)
        .args(manager.iter())
        .arg(package)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        // 126/127: o usuário cancelou a senha ou não tem permissão.
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { "A instalação foi cancelada.".into() } else { err })
    }
}

/// Fecha o Medalha e abre a versão nova assim que este processo terminar
/// (enquanto ele vive, a instância nova só repassaria os argumentos e sairia).
fn relaunch(app: &AppHandle, file: &Path) -> Result<(), String> {
    let pid = std::process::id().to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Espera o processo sumir e abre o instalador (que reabre o app no fim).
        let script = format!(
            "while (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ Start-Sleep -Milliseconds 300 }}; Start-Process -FilePath '{}'",
            file.display().to_string().replace('\'', "''")
        );
        command("powershell.exe")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command"])
            .raw_arg(format!("\"{script}\""))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        let target = match std::env::var_os("APPIMAGE") {
            Some(appimage) => std::path::PathBuf::from(appimage),
            None => std::path::PathBuf::from("/usr/bin/medalha"),
        };
        let _ = file;
        command("sh")
            .args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.3; done; exec \"$2\" --show", "sh"])
            .arg(&pid)
            .arg(&target)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    crate::quit(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert_eq!(super::version("v0.2.2"), Some((0, 2, 2)));
        assert_eq!(super::version("1.10.0"), Some((1, 10, 0)));
        assert!(super::version("v0.10.0") > super::version("v0.9.9"));
        assert_eq!(super::version("nightly"), None);
    }
}
