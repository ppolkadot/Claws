use tauri::WebviewWindow;

/// "glass" uses the accent policy, the only API that blurs the desktop without
/// Acrylic's tint and noise; "acrylic", "mica" and "tabbed" are DWM system backdrops.
#[cfg(windows)]
pub fn apply_backdrop(window: &WebviewWindow, kind: &str) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
    };
    use windows::Win32::UI::Controls::MARGINS;

    // DWMWA_WINDOW_CORNER_PREFERENCE and DWMWA_SYSTEMBACKDROP_TYPE.
    const CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
    const SYSTEM_BACKDROP_TYPE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(38);
    const CORNER_ROUND: i32 = 2;

    let backdrop: i32 = match kind {
        "mica" => 2,
        "acrylic" => 3,
        "tabbed" => 4,
        _ => 1, // none, also for glass, which supplies its own blur
    };
    // The frame has to cover the whole client area for any material, glass included,
    // or DWM leaves the window see-through with nothing drawn behind it.
    let margin = if kind == "none" { 0 } else { -1 };

    let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
    unsafe {
        let margins = MARGINS {
            cxLeftWidth: margin,
            cxRightWidth: margin,
            cyTopHeight: margin,
            cyBottomHeight: margin,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins).map_err(|e| e.to_string())?;
        DwmSetWindowAttribute(
            hwnd,
            SYSTEM_BACKDROP_TYPE,
            &backdrop as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
        .map_err(|e| e.to_string())?;
        // Rounded corners are best effort: older builds simply ignore the call.
        let _ = DwmSetWindowAttribute(
            hwnd,
            CORNER_PREFERENCE,
            &CORNER_ROUND as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }

    // ACCENT_ENABLE_BLURBEHIND for glass, ACCENT_DISABLED otherwise so switching
    // materials never stacks two blurs.
    set_accent(hwnd, if kind == "glass" { 3 } else { 0 });
    Ok(())
}

/// SetWindowCompositionAttribute is exported by user32 but missing from its import
/// library, so it is looked up at runtime. Failure only means no blur.
#[cfg(windows)]
fn set_accent(hwnd: windows::Win32::Foundation::HWND, state: u32) {
    use std::ffi::c_void;
    use windows::core::{s, BOOL};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut c_void,
        size: usize,
    }
    const WCA_ACCENT_POLICY: u32 = 19;
    type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

    unsafe {
        let Ok(user32) = GetModuleHandleA(s!("user32.dll")) else { return };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
            return;
        };
        let set: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            accent_state: state,
            accent_flags: 0,
            gradient_color: 0,
            animation_id: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut AccentPolicy as *mut c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        let _ = set(hwnd, &mut data);
    }
}

#[cfg(not(windows))]
pub fn apply_backdrop(_window: &WebviewWindow, _kind: &str) -> Result<(), String> {
    Ok(())
}

/// The native theme also decides whether the system materials are tinted light or dark.
pub fn apply_theme(window: &WebviewWindow, theme: &str) {
    let theme = match theme {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    let _ = window.set_theme(theme);
}
