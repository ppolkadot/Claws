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
        let system = DwmSetWindowAttribute(
            hwnd,
            SYSTEM_BACKDROP_TYPE,
            &backdrop as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // Builds before Windows 11 22H2 lack this attribute. That only matters when a
        // system material was asked for; glass and solid must still be applied below.
        if system.is_err() && backdrop != 1 {
            set_accent(hwnd, 0);
            return Err(
                "This version of Windows does not support Acrylic or Mica. Choose Glass instead.".into(),
            );
        }
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
    let applied = set_accent(hwnd, if kind == "glass" { 3 } else { 0 });
    if kind == "glass" && !applied {
        return Err("Windows did not turn on the desktop blur for Claws. Check that Transparency effects is on in Windows Settings → Personalization → Colors.".into());
    }
    Ok(())
}

/// SetWindowCompositionAttribute is exported by user32 but missing from its import
/// library, so it is looked up at runtime. False means no blur.
#[cfg(windows)]
fn set_accent(hwnd: windows::Win32::Foundation::HWND, state: u32) -> bool {
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
        let Ok(user32) = GetModuleHandleA(s!("user32.dll")) else { return false };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
            return false;
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
        set(hwnd, &mut data).as_bool()
    }
}

/// Fallback when Windows refuses the focus change (it only allows it to the process
/// that received the last input): attaches to the foreground thread's input for the
/// switch. A no-op when Claws is already in front.
#[cfg(windows)]
pub fn take_foreground(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    let Ok(raw) = window.hwnd() else { return };
    let hwnd = HWND(raw.0);
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == hwnd {
            return;
        }
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(foreground, None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
    }
}

#[cfg(not(windows))]
pub fn take_foreground(_window: &WebviewWindow) {}

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
