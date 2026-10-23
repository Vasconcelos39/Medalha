//! Webview do medal.tv embutido abaixo do cabeçalho da interface.

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl, Window};

pub const LABEL: &str = "medal";
pub const HOME: &str = "https://medal.tv/";
/// Altura do cabeçalho em `ui/style.css` (--header-h).
pub const HEADER_H: f64 = 52.0;

pub fn builder<R: Runtime>() -> WebviewBuilder<R> {
    WebviewBuilder::new(LABEL, WebviewUrl::External(HOME.parse().expect("URL válida")))
}

/// Ocupa toda a área abaixo do cabeçalho.
pub fn layout<R: Runtime>(window: &Window<R>) {
    let Some(webview) = window.app_handle().get_webview(LABEL) else { return };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else { return };
    let size = size.to_logical::<f64>(scale);
    let _ = webview.set_position(LogicalPosition::new(0.0, HEADER_H));
    let _ = webview.set_size(LogicalSize::new(size.width, (size.height - HEADER_H).max(1.0)));
}

pub fn set_visible<R: Runtime>(app: &AppHandle<R>, visible: bool) {
    if let Some(webview) = app.get_webview(LABEL) {
        let _ = if visible { webview.show() } else { webview.hide() };
    }
}

pub fn navigate<R: Runtime>(app: &AppHandle<R>, action: &str) -> Result<(), String> {
    let webview = app.get_webview(LABEL).ok_or("webview do Medal não existe")?;
    let script = match action {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        "home" => "location.href = 'https://medal.tv/'",
        _ => return Err(format!("ação desconhecida: {action}")),
    };
    webview.eval(script).map_err(|e| e.to_string())
}
