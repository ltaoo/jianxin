use gpui_kit::{point, px, size, App, Bounds, DisplayId, Pixels};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn panel_rect(work: Rect, anchor: Option<Rect>, width: f32, height: f32, gap: f32) -> Rect {
    let width = width.min(work.width);
    let height = height.min(work.height);
    let (x, y) = match anchor {
        Some(anchor) => {
            let below = anchor.y + anchor.height + gap;
            let y = if below + height <= work.y + work.height {
                below
            } else {
                anchor.y - height - gap
            };
            (anchor.x + anchor.width / 2. - width / 2., y)
        }
        None => (work.x + work.width - width - gap, work.y + gap),
    };
    Rect {
        x: x.clamp(work.x, work.x + work.width - width),
        y: y.clamp(work.y, work.y + work.height - height),
        width,
        height,
    }
}

fn from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
    }
}

pub fn placement(
    cx: &App,
    anchor: Option<tray_icon::Rect>,
    width: f32,
    height: f32,
    gap: f32,
) -> (Bounds<Pixels>, Option<DisplayId>) {
    let native = anchor.and_then(native_anchor);
    let display = native
        .as_ref()
        .and_then(|(_, _, id)| {
            cx.displays()
                .into_iter()
                .find(|display| display.id() == *id)
        })
        .or_else(|| cx.primary_display());
    let work = native
        .as_ref()
        .map(|(_, work, _)| *work)
        .or_else(|| display.as_ref().map(|d| from_bounds(d.visible_bounds())))
        .unwrap_or(Rect {
            x: 0.,
            y: 0.,
            width: 1024.,
            height: 768.,
        });
    let rect = panel_rect(
        work,
        native.map(|(anchor, _, _)| anchor),
        width,
        height,
        gap,
    );
    (
        Bounds::new(
            point(px(rect.x), px(rect.y)),
            size(px(rect.width), px(rect.height)),
        ),
        display.map(|d| d.id()),
    )
}

#[cfg(target_os = "macos")]
fn native_anchor(rect: tray_icon::Rect) -> Option<(Rect, Rect, DisplayId)> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen};
    let marker = MainThreadMarker::new()?;
    let screens = NSScreen::screens(marker);
    let primary_height = screens.firstObject()?.frame().size.height;
    let cursor = NSEvent::mouseLocation();
    for screen in screens.iter() {
        let scale = screen.backingScaleFactor() as f32;
        let frame = screen.frame();
        let bounds = Rect {
            x: frame.origin.x as f32,
            y: (primary_height - frame.origin.y - frame.size.height) as f32,
            width: frame.size.width as f32,
            height: frame.size.height as f32,
        };
        let anchor = Rect {
            x: rect.position.x as f32 / scale,
            y: rect.position.y as f32 / scale,
            width: rect.size.width as f32 / scale,
            height: rect.size.height as f32 / scale,
        };
        if cursor.x >= frame.origin.x
            && cursor.x < frame.origin.x + frame.size.width
            && cursor.y >= frame.origin.y
            && cursor.y < frame.origin.y + frame.size.height
        {
            let visible = screen.visibleFrame();
            let description = screen.deviceDescription();
            let number =
                description.objectForKey(objc2_foundation::ns_string!("NSScreenNumber"))?;
            let number: u32 = unsafe { objc2::msg_send![&*number, unsignedIntValue] };
            // GPUI macOS bounds are relative to the selected display, unlike Windows.
            let anchor = Rect {
                x: anchor.x - bounds.x,
                y: anchor.y - bounds.y,
                ..anchor
            };
            return Some((
                anchor,
                Rect {
                    x: (visible.origin.x - frame.origin.x) as f32,
                    y: (frame.origin.y + frame.size.height - visible.origin.y - visible.size.height)
                        as f32,
                    width: visible.size.width as f32,
                    height: visible.size.height as f32,
                },
                DisplayId::new(number as u64),
            ));
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn native_anchor(rect: tray_icon::Rect) -> Option<(Rect, Rect, DisplayId)> {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    };
    let native = RECT {
        left: rect.position.x as i32,
        top: rect.position.y as i32,
        right: rect.position.x as i32 + rect.size.width as i32,
        bottom: rect.position.y as i32 + rect.size.height as i32,
    };
    // Native monitor APIs return physical coordinates; GPUI expects logical pixels.
    unsafe {
        let monitor = MonitorFromRect(&native, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let (mut dx, mut dy) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) < 0 {
            dx = 96;
        }
        let scale = dx.max(1) as f32 / 96.;
        Some((
            Rect {
                x: native.left as f32 / scale,
                y: native.top as f32 / scale,
                width: rect.size.width as f32 / scale,
                height: rect.size.height as f32 / scale,
            },
            Rect {
                x: info.rcWork.left as f32 / scale,
                y: info.rcWork.top as f32 / scale,
                width: (info.rcWork.right - info.rcWork.left) as f32 / scale,
                height: (info.rcWork.bottom - info.rcWork.top) as f32 / scale,
            },
            DisplayId::new(monitor as usize as u64),
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_anchor(_: tray_icon::Rect) -> Option<(Rect, Rect, DisplayId)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_handles_top_bottom_edges_negative_monitors_and_small_screens() {
        let work = Rect {
            x: -1280.,
            y: 24.,
            width: 1280.,
            height: 1000.,
        };
        let top = panel_rect(
            work,
            Some(Rect {
                x: -25.,
                y: 0.,
                width: 22.,
                height: 24.,
            }),
            384.,
            560.,
            8.,
        );
        assert_eq!(top.x, -384.);
        assert_eq!(top.y, 32.);
        let bottom = panel_rect(
            work,
            Some(Rect {
                x: -1250.,
                y: 1024.,
                width: 22.,
                height: 24.,
            }),
            384.,
            560.,
            8.,
        );
        assert_eq!(bottom.x, -1280.);
        assert_eq!(bottom.y, 456.);
        let tiny = panel_rect(
            Rect {
                x: 0.,
                y: 0.,
                width: 200.,
                height: 300.,
            },
            None,
            384.,
            560.,
            8.,
        );
        assert_eq!(
            (tiny.x, tiny.y, tiny.width, tiny.height),
            (0., 0., 200., 300.)
        );
    }
}
