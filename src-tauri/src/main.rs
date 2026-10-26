#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod clips;
mod config;
mod game;
mod media;
mod medal;
mod recorder;
mod sound;
mod util;

use config::Config;
use recorder::Recorder;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, RunEvent, WebviewUrl, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub recorder: Recorder,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

type CmdResult<T> = Result<T, String>;

fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

/// Roda fora da thread principal (parar o gravador pode levar alguns segundos).
fn in_background(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || f(&app));
}

fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = CmdResult<T>> {
    let handle = tauri::async_runtime::spawn_blocking(f);
    async move { handle.await.map_err(|e| e.to_string()) }
}

// Comandos chamados pela interface.

#[tauri::command]
fn get_status(app: AppHandle) -> recorder::Status {
    state(&app).recorder.status()
}

#[tauri::command]
fn get_config(app: AppHandle) -> Config {
    state(&app).config.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(app: AppHandle, mut config: Config) -> CmdResult<Config> {
    config.sanitize();
    if config.clips_dir.as_os_str().is_empty() {
        return Err("Escolha uma pasta de clipes.".into());
    }
    let st = state(&app);
    let old = st.config.lock().unwrap().clone();
    if old.autostart != config.autostart {
        autostart::set(config.autostart)?;
    }
    *st.config.lock().unwrap() = config.clone();
    config.save(&st.config_path)?;
    if old.hotkey != config.hotkey {
        register_hotkey(&app, &config.hotkey);
    }
    if old.clips_dir != config.clips_dir {
        apply_clips_dir(&app, &config.clips_dir);
        let _ = app.emit("clips-changed", ());
    }
    if old.recording_changed(&config) {
        in_background(&app, |app| state(app).recorder.restart(app));
    }
    Ok(config)
}

#[tauri::command]
fn start_buffer(app: AppHandle) {
    state(&app).recorder.start(&app);
}

#[tauri::command]
fn stop_buffer(app: AppHandle) {
    in_background(&app, |app| state(app).recorder.stop(app));
}

#[tauri::command]
fn save_clip(app: AppHandle) {
    state(&app).recorder.save(&app);
}

#[tauri::command]
async fn list_clips(app: AppHandle) -> CmdResult<Vec<clips::ClipInfo>> {
    let dir = state(&app).config.lock().unwrap().clips_dir.clone();
    blocking(move || clips::list(&dir)).await
}

#[tauri::command]
async fn clip_meta(app: AppHandle, path: String) -> CmdResult<clips::ClipMeta> {
    let (ffmpeg_path, cache_dir) = {
        let st = state(&app);
        let cfg = st.config.lock().unwrap();
        (cfg.ffmpeg_path.clone(), st.cache_dir.clone())
    };
    blocking(move || {
        let ffmpeg = media::find_ffmpeg(&ffmpeg_path);
        clips::meta(ffmpeg.as_deref(), &cache_dir, Path::new(&path))
    })
    .await
}

fn clip_path(app: &AppHandle, path: &str) -> CmdResult<PathBuf> {
    let dir = state(app).config.lock().unwrap().clips_dir.clone();
    clips::ensure_inside(&dir, path)
}

#[tauri::command]
async fn delete_clip(app: AppHandle, path: String) -> CmdResult<()> {
    let path = clip_path(&app, &path)?;
    blocking(move || clips::trash(&path)).await?
}

#[tauri::command]
async fn trim_clip(app: AppHandle, path: String, start: f64, end: f64) -> CmdResult<clips::ClipInfo> {
    let path = clip_path(&app, &path)?;
    let ffmpeg_path = state(&app).config.lock().unwrap().ffmpeg_path.clone();
    blocking(move || {
        let ffmpeg = media::find_ffmpeg(&ffmpeg_path).ok_or("ffmpeg não encontrado.")?;
        clips::trim(&ffmpeg, &path, start, end)
    })
    .await?
}

#[tauri::command]
fn rename_clip(app: AppHandle, path: String, name: String) -> CmdResult<String> {
    let path = clip_path(&app, &path)?;
    let renamed = clips::rename(&path, &name)?;
    Ok(renamed.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

#[tauri::command]
fn reveal_clip(app: AppHandle, path: String) -> CmdResult<()> {
    clips::reveal(&clip_path(&app, &path)?);
    Ok(())
}

#[tauri::command]
fn open_clip(app: AppHandle, path: String) -> CmdResult<()> {
    clips::open_path(&clip_path(&app, &path)?);
    Ok(())
}

#[tauri::command]
fn open_clips_dir(app: AppHandle) {
    let dir = state(&app).config.lock().unwrap().clips_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    clips::open_path(&dir);
}

/// Baixa o gpu-screen-recorder do Flathub e liga o buffer.
#[tauri::command]
async fn install_gsr(app: AppHandle) -> CmdResult<()> {
    blocking(recorder::install_gsr).await??;
    state(&app).recorder.start(&app);
    Ok(())
}

/// Solta o atalho global enquanto o usuário grava um novo nas configurações;
/// senão, apertar o atalho atual salvaria um clipe em vez de ser capturado.
#[tauri::command]
fn suspend_hotkey(app: AppHandle, suspended: bool) {
    if suspended {
        let _ = app.global_shortcut().unregister_all();
    } else {
        let hotkey = state(&app).config.lock().unwrap().hotkey.clone();
        register_hotkey(&app, &hotkey);
    }
}

#[tauri::command]
fn set_medal_visible(app: AppHandle, visible: bool) {
    medal::set_visible(&app, visible);
}

#[tauri::command]
fn medal_nav(app: AppHandle, action: String) -> CmdResult<()> {
    medal::navigate(&app, &action)
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    blocking(move || {
        app.dialog()
            .file()
            .set_title("Pasta dos clipes")
            .blocking_pick_folder()
            .and_then(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostics {
    version: String,
    platform: String,
    ffmpeg: Option<String>,
    gsr: Option<String>,
    backend: String,
}

#[tauri::command]
async fn diagnostics(app: AppHandle) -> CmdResult<Diagnostics> {
    let cfg = state(&app).config.lock().unwrap().clone();
    let version = app.package_info().version.to_string();
    blocking(move || {
        #[cfg(target_os = "linux")]
        let gsr = recorder::gsr::find().map(|g| g.describe());
        #[cfg(not(target_os = "linux"))]
        let gsr = None;
        Diagnostics {
            version,
            platform: format!(
                "{} ({}){}",
                std::env::consts::OS,
                util::session_kind(),
                if std::env::var_os("APPIMAGE").is_some() { " · AppImage" } else { "" }
            ),
            ffmpeg: media::find_ffmpeg(&cfg.ffmpeg_path).map(|p| p.display().to_string()),
            gsr,
            backend: recorder::backend_label(&cfg),
        }
    })
    .await
}

// Integração com o sistema.

fn apply_clips_dir(app: &AppHandle, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let _ = app.asset_protocol_scope().allow_directory(dir, true);
    *state(app).watcher.lock().unwrap() = clips::watch(app, dir);
}

fn register_hotkey(app: &AppHandle, hotkey: &str) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    // No Wayland isso costuma falhar; aí o usuário associa `medalha --save-clip` a um atalho do sistema.
    let ok = hotkey
        .parse::<Shortcut>()
        .map(|s| shortcuts.register(s).is_ok())
        .unwrap_or(false);
    state(app).recorder.set_hotkey(app, hotkey, ok);
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit(app: &AppHandle) {
    in_background(app, |app| {
        state(app).recorder.stop(app);
        app.exit(0);
    });
}

/// `medalha --save-clip | --start | --stop | --toggle | --show`
fn handle_cli(app: &AppHandle, args: &[String], second_instance: bool) {
    let mut acted = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--save-clip" | "--save" => state(app).recorder.save(app),
            "--start" => state(app).recorder.start(app),
            "--stop" => in_background(app, |app| state(app).recorder.stop(app)),
            "--toggle" => in_background(app, |app| state(app).recorder.toggle(app)),
            "--show" => show_window(app),
            _ => continue,
        }
        acted = true;
    }
    if second_instance && !acted {
        show_window(app);
    }
}

fn build_window(app: &tauri::App, hidden: bool) -> tauri::Result<()> {
    let mut builder = tauri::window::WindowBuilder::new(app, "main")
        .title("Medalha")
        .inner_size(1280.0, 820.0)
        .min_inner_size(960.0, 600.0)
        .visible(!hidden);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone())?;
    }
    let window = builder.build()?;
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    window.add_child(
        WebviewBuilder::new(medal::UI_LABEL, WebviewUrl::App("index.html".into())).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        size,
    )?;
    window.add_child(
        medal::builder(),
        LogicalPosition::new(0.0, medal::HEADER_H),
        LogicalSize::new(size.width, (size.height - medal::HEADER_H).max(1.0)),
    )?;
    medal::set_visible(window.app_handle(), true);

    let w = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => medal::layout(&w),
        WindowEvent::CloseRequested { api, .. } => {
            let to_tray = w.app_handle().state::<AppState>().config.lock().unwrap().close_to_tray;
            if to_tray {
                api.prevent_close();
                let _ = w.hide();
            }
        }
        _ => {}
    });
    Ok(())
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Abrir Medalha", true, None::<&str>)?;
    let save = MenuItem::with_id(app, "save", "Salvar clipe", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Ligar/desligar buffer", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let items: [&dyn IsMenuItem<tauri::Wry>; 5] = [&show, &save, &toggle, &sep, &quit_item];
    let menu = Menu::with_items(app, &items)?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Medalha")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "save" => state(app).recorder.save(app),
            "toggle" => in_background(app, |app| state(app).recorder.toggle(app)),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hidden = args.iter().any(|a| a == "--hidden");

    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // Evita tela branca do WebKitGTK com drivers NVIDIA.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| handle_cli(app, &argv, true)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        state(app).recorder.save(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_config,
            set_config,
            start_buffer,
            stop_buffer,
            save_clip,
            list_clips,
            clip_meta,
            delete_clip,
            trim_clip,
            rename_clip,
            reveal_clip,
            open_clip,
            open_clips_dir,
            install_gsr,
            suspend_hotkey,
            set_medal_visible,
            medal_nav,
            pick_folder,
            diagnostics,
        ])
        .setup(move |app| {
            let paths = app.path();
            let config_path = paths.app_config_dir()?.join("config.json");
            let cache_dir = paths.app_cache_dir()?;
            let default_clips = paths
                .video_dir()
                .or_else(|_| paths.home_dir().map(|h| h.join("Videos")))?
                .join("Medalha");
            let config = Config::load(&config_path, default_clips);
            let _ = std::fs::create_dir_all(cache_dir.join("thumbs"));
            let _ = app.asset_protocol_scope().allow_directory(cache_dir.join("thumbs"), true);
            let start_now = config.start_on_launch;
            if config.autostart {
                // Reescreve o caminho do executável (mudou se o app foi reinstalado ou o AppImage movido).
                std::thread::spawn(|| {
                    let _ = autostart::set(true);
                });
            }
            let hotkey = config.hotkey.clone();
            let clips_dir = config.clips_dir.clone();
            app.manage(AppState {
                config: Mutex::new(config),
                config_path,
                cache_dir,
                recorder: Recorder::new(),
                watcher: Mutex::new(None),
            });

            let handle = app.handle().clone();
            apply_clips_dir(&handle, &clips_dir);
            register_hotkey(&handle, &hotkey);
            build_window(app, hidden)?;
            build_tray(app)?;
            if start_now {
                state(&handle).recorder.start(&handle);
            }
            handle_cli(&handle, &args, false);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o Medalha");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            state(app).recorder.stop(app);
        }
    });
}
