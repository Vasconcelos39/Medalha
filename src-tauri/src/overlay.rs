//! Card "Clipe salvo" por cima do jogo, como o do Medal.
//!
//! É uma janela sem borda, transparente, sempre no topo, que não pega foco nem
//! clique (o jogo não pode perder o foco). No Wayland um app comum não escolhe a
//! posição da janela nem fica acima de tela cheia; lá o card usa o protocolo
//! layer-shell (KDE, Hyprland, Sway…) pela libgtk-layer-shell, carregada em tempo
//! de execução. Sem ela, ou no GNOME, que não tem o protocolo, `show` devolve
//! `false` e quem chamou usa a notificação do sistema.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LABEL: &str = "overlay";
const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 76.0;
const MARGIN: i32 = 24;
const VISIBLE_FOR: Duration = Duration::from_millis(3500);

/// Cada card mostrado ganha um número; o timer só esconde se nenhum outro veio depois.
static SHOWN: AtomicU64 = AtomicU64::new(0);
static LOADED: AtomicBool = AtomicBool::new(false);

/// Mostra o card. Chame fora da thread principal (espera a janela ficar pronta).
pub fn show(app: &AppHandle, title: &str, subtitle: &str, kind: &str) -> bool {
    if !supported(app) {
        return false;
    }
    let window = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => match create(app) {
            Some(w) => w,
            None => return false,
        },
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while !LOADED.load(Ordering::SeqCst) {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    place(&window);
    let card = serde_json::json!({ "title": title, "subtitle": subtitle, "kind": kind });
    if window.eval(format!("showCard({card})")).is_err() || window.show().is_err() {
        return false;
    }
    let id = SHOWN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(VISIBLE_FOR);
        if SHOWN.load(Ordering::SeqCst) != id {
            return;
        }
        let _ = window.eval("hideCard()");
        std::thread::sleep(Duration::from_millis(250));
        if SHOWN.load(Ordering::SeqCst) == id {
            let _ = window.hide();
        }
    });
    true
}

fn create(app: &AppHandle) -> Option<WebviewWindow> {
    crate::logfile::write("card: criando a janela");
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Medalha")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .on_page_load(|_, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                LOADED.store(true, Ordering::SeqCst);
            }
        })
        .build()
        .ok()?;
    let _ = window.set_ignore_cursor_events(true);
    #[cfg(target_os = "linux")]
    if crate::util::is_wayland() && !layer::attach(app, &window) {
        crate::logfile::write("card: layer-shell falhou, usando a notificação");
        let _ = window.destroy();
        return None;
    }
    crate::logfile::write("card: janela pronta");
    Some(window)
}

/// Canto superior direito do monitor principal. No Wayland quem posiciona é o layer-shell.
fn place(window: &WebviewWindow) {
    if crate::util::is_wayland() {
        return;
    }
    let Ok(Some(monitor)) = window.primary_monitor() else { return };
    let scale = monitor.scale_factor();
    let (pos, size) = (monitor.position(), monitor.size());
    let x = pos.x + size.width as i32 - ((WIDTH + MARGIN as f64) * scale) as i32;
    let y = pos.y + (MARGIN as f64 * scale) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(target_os = "linux")]
fn supported(app: &AppHandle) -> bool {
    !crate::util::is_wayland() || layer::available(app)
}

#[cfg(not(target_os = "linux"))]
fn supported(_app: &AppHandle) -> bool {
    cfg!(windows)
}

/// libgtk-layer-shell carregada com dlopen: é opcional, então o Medalha abre mesmo
/// sem ela (só não mostra o card no Wayland).
#[cfg(target_os = "linux")]
mod layer {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::sync::mpsc;
    use std::sync::OnceLock;
    use std::time::Duration;
    use tauri::{AppHandle, WebviewWindow};

    type WindowFn = unsafe extern "C" fn(*mut c_void);
    type IntFn = unsafe extern "C" fn(*mut c_void, c_int);
    type EdgeFn = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type NamespaceFn = unsafe extern "C" fn(*mut c_void, *const c_char);
    type SupportedFn = unsafe extern "C" fn() -> c_int;

    const LAYER_OVERLAY: c_int = 3;
    const EDGE_RIGHT: c_int = 1;
    const EDGE_TOP: c_int = 2;

    struct Lib {
        init_for_window: WindowFn,
        set_layer: IntFn,
        set_anchor: EdgeFn,
        set_margin: EdgeFn,
        set_namespace: Option<NamespaceFn>,
        /// Só existe a partir da 0.6; nas anteriores supomos que o compositor tem o protocolo.
        is_supported: Option<SupportedFn>,
    }

    // Os ponteiros são de funções de uma biblioteca que nunca é descarregada.
    unsafe impl Send for Lib {}
    unsafe impl Sync for Lib {}

    unsafe fn sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
        let ptr = libc::dlsym(handle, name.as_ptr());
        (!ptr.is_null()).then(|| std::mem::transmute_copy(&ptr))
    }

    fn lib() -> Option<&'static Lib> {
        static LIB: OnceLock<Option<Lib>> = OnceLock::new();
        LIB.get_or_init(|| unsafe {
            let handle = libc::dlopen(c"libgtk-layer-shell.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                return None;
            }
            Some(Lib {
                init_for_window: sym(handle, c"gtk_layer_init_for_window")?,
                set_layer: sym(handle, c"gtk_layer_set_layer")?,
                set_anchor: sym(handle, c"gtk_layer_set_anchor")?,
                set_margin: sym(handle, c"gtk_layer_set_margin")?,
                set_namespace: sym(handle, c"gtk_layer_set_namespace"),
                is_supported: sym(handle, c"gtk_layer_is_supported"),
            })
        })
        .as_ref()
    }

    /// Roda `f` na thread do GTK e espera o resultado.
    fn on_main<T: Send + 'static>(app: &AppHandle, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (tx, rx) = mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .ok()?;
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    pub fn available(app: &AppHandle) -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            let Some(lib) = lib() else { return false };
            match lib.is_supported {
                Some(is_supported) => on_main(app, move || unsafe { is_supported() != 0 }).unwrap_or(false),
                None => true,
            }
        })
    }

    /// Transforma a janela (ainda nunca mostrada) numa superfície layer-shell no canto superior direito.
    pub fn attach(app: &AppHandle, window: &WebviewWindow) -> bool {
        let Some(lib) = lib() else { return false };
        let window = window.clone();
        on_main(app, move || {
            use gtk::glib::object::ObjectType;
            let Ok(gtk_window) = window.gtk_window() else { return false };
            let ptr = gtk_window.as_ptr() as *mut c_void;
            unsafe {
                (lib.init_for_window)(ptr);
                (lib.set_layer)(ptr, LAYER_OVERLAY);
                (lib.set_anchor)(ptr, EDGE_TOP, 1);
                (lib.set_anchor)(ptr, EDGE_RIGHT, 1);
                (lib.set_margin)(ptr, EDGE_TOP, super::MARGIN);
                (lib.set_margin)(ptr, EDGE_RIGHT, super::MARGIN);
                if let Some(set_namespace) = lib.set_namespace {
                    set_namespace(ptr, c"medalha".as_ptr());
                }
            }
            true
        })
        .unwrap_or(false)
    }
}
