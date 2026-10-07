//! AppKit menu bar artwork and local visibility diagnostics.
use anyhow::{Context, Result};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSCellImagePosition, NSImage};
use objc2_foundation::{NSCopying, NSSize, NSString};
use tray_icon::TrayIcon;

pub fn configure(tray: &TrayIcon) -> Result<()> {
    let marker =
        MainThreadMarker::new().context("menu bar must be configured on the main thread")?;
    let item = tray
        .ns_status_item()
        .context("menu bar item was not created")?;
    let button = item
        .button(marker)
        .context("menu bar button was not created")?;
    let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("envelope"),
        Some(&NSString::from_str("简信未读邮件")),
    )
    .context("cannot load macOS envelope symbol")?;
    let symbol = symbol.copy();
    symbol.setSize(NSSize::new(18.0, 18.0));
    symbol.setTemplate(true);
    button.setImage(Some(&symbol));
    button.setImagePosition(NSCellImagePosition::ImageLeft);
    item.setVisible(true);
    Ok(())
}

pub fn record_status(tray: &TrayIcon, path: &std::path::Path) {
    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let Some(item) = tray.ns_status_item() else {
        return;
    };
    let button = item.button(marker);
    let frame = button.as_ref().and_then(|button| button.window()).map(|window| {
        let frame = window.frame();
        serde_json::json!({"x": frame.origin.x, "y": frame.origin.y, "width": frame.size.width, "height": frame.size.height})
    });
    let status = serde_json::json!({
        "pid": std::process::id(),
        "executable": std::env::current_exe().ok(),
        "visible": item.isVisible(),
        "has_image": button.as_ref().is_some_and(|button| button.image().is_some()),
        "button_hidden": button.as_ref().map(|button| button.isHidden()),
        "frame": frame,
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&status) {
        let _ = std::fs::write(path, bytes);
    }
}
