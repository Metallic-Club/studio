//! What Studio asks of the operating system directly: the splash window's shape and the system's reduced-motion
//! switch. Windows only for now; elsewhere these do nothing.
//!
//! The shape: the engine's surfaces present opaque (`RenderSurface` picks an opaque alpha mode, the only one a
//! Direct3D 12 window swapchain has), so the transparent clear around the splash's rounded corners would show black. On Windows the window's region clips it to the splash's outline instead. Other systems show the clear
//! colour there until the render surface can present with alpha.

use egui::Pos2;
use winit::window::Window;

/// Clips `window` to `outline` (convex, in physical pixels from the window's top-left), or back to its rectangle
/// with `None`.
#[cfg(windows)]
pub(crate) fn set_window_shape(window: &Window, outline: Option<&[Pos2]>) {
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::{CreatePolygonRgn, DeleteObject, HRGN, SetWindowRgn, WINDING};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let region: HRGN = match outline {
        Some(outline) => {
            let points: Vec<POINT> = outline
                .iter()
                .map(|p| POINT {
                    x: p.x.round() as i32,
                    y: p.y.round() as i32,
                })
                .collect();
            let Ok(count) = i32::try_from(points.len()) else {
                return;
            };
            // SAFETY: `points` holds `count` initialised POINTs and outlives the call, which copies them.
            let region = unsafe { CreatePolygonRgn(points.as_ptr(), count, WINDING) };
            if region.is_null() {
                log::warn!("splash: could not make the window's shape");
                return;
            }
            region
        }
        None => std::ptr::null_mut(),
    };
    // SAFETY: `hwnd` is the live window `window` owns, on the thread that made it (winit's event loop). A null
    // region removes the shape. On success the system owns `region`; on failure it is still ours and deleted below.
    let set = unsafe { SetWindowRgn(hwnd, region, 1) };
    if set == 0 && !region.is_null() {
        log::warn!("splash: could not shape the window");
        // SAFETY: the region was made above and the system refused it, so nothing else holds it.
        unsafe { DeleteObject(region) };
    }
}

/// Clips `window` to `outline` (convex, in physical pixels from the window's top-left), or back to its rectangle
/// with `None`. Nothing to do on this system: see the module docs.
#[cfg(not(windows))]
pub(crate) fn set_window_shape(_window: &Window, _outline: Option<&[Pos2]>) {}

/// Whether the system asks for less motion: Windows' "Animation effects" switched off.
#[cfg(windows)]
pub(crate) fn prefers_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};

    let mut animate: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL (an i32) through the pointer, which points at `animate`.
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0) };
    read != 0 && animate == 0
}

/// Whether the system asks for less motion. Not read on this system yet: Studio's own setting decides.
#[cfg(not(windows))]
pub(crate) fn prefers_reduced_motion() -> bool {
    false
}
