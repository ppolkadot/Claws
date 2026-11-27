//! The few Windows window calls the Wallpaper Engine recorder needs: find the pop-out window
//! Wallpaper Engine opened for us, put it where nobody sees it, read its size, hand keyboard
//! focus back, and close it. Windows are named by their handle as a plain number so the rest of
//! Claws never touches a raw `HWND`.
//!
//! The window is only ever found by its exact title *and* by the program that owns it, so a
//! window of another program with the same title is never touched.
use std::path::Path;

/// `a` and `b` name the same program file: compared without case, with one kind of separator
/// and without the `\\?\` prefix that `canonicalize` adds.
pub fn same_image(a: &str, b: &str) -> bool {
    fn normal(path: &str) -> String {
        path.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase()
    }
    !a.is_empty() && normal(a) == normal(b)
}

/// A window is ours when its title is exactly the one we asked Wallpaper Engine to use and the
/// program that owns it is Wallpaper Engine's own executable.
pub fn is_ours(title: &str, expected_title: &str, image: &str, engine: &Path) -> bool {
    title == expected_title && same_image(image, &engine.to_string_lossy())
}

/// What the display allows, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Display {
    /// The largest window a framed window can have (`SM_CXMAXTRACK` by `SM_CYMAXTRACK`).
    pub max_client: (u32, u32),
    /// The primary monitor.
    pub primary: (u32, u32),
    /// The right edge of the virtual screen: a window placed here is beyond every monitor.
    pub right_edge: i32,
}

#[cfg(windows)]
mod imp {
    use super::Display;
    use std::path::Path;
    use windows::core::{BOOL, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
        PostMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, HWND_BOTTOM, SM_CXMAXTRACK, SM_CXSCREEN,
        SM_CXVIRTUALSCREEN, SM_CYMAXTRACK, SM_CYSCREEN, SM_XVIRTUALSCREEN, SWP_FRAMECHANGED, SWP_NOACTIVATE, WM_CLOSE,
    };

    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_THICKFRAME: u32 = 0x0004_0000;
    const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const WS_EX_APPWINDOW: u32 = 0x0004_0000;

    fn hwnd(handle: isize) -> HWND {
        HWND(handle as *mut core::ffi::c_void)
    }

    pub fn foreground() -> Option<isize> {
        let window = unsafe { GetForegroundWindow() };
        (!window.0.is_null()).then_some(window.0 as isize)
    }

    /// Hands keyboard focus back to a window, if it still exists. Best effort: Windows may
    /// refuse, and the window then simply stays behind.
    pub fn restore_foreground(handle: isize) {
        unsafe {
            if IsWindow(Some(hwnd(handle))).as_bool() {
                let _ = SetForegroundWindow(hwnd(handle));
            }
        }
    }

    pub fn display() -> Option<Display> {
        let metric = |index| u32::try_from(unsafe { GetSystemMetrics(index) }).ok().filter(|value| *value > 0);
        let virtual_left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let virtual_width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        Some(Display {
            max_client: (metric(SM_CXMAXTRACK)?, metric(SM_CYMAXTRACK)?),
            primary: (metric(SM_CXSCREEN)?, metric(SM_CYSCREEN)?),
            right_edge: virtual_left.saturating_add(virtual_width),
        })
    }

    struct Search<'a> {
        title: &'a str,
        engine: &'a Path,
        found: Option<isize>,
    }

    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut buffer = [0u16; 512];
        let length = GetWindowTextW(window, &mut buffer);
        if length <= 0 {
            return BOOL(1);
        }
        let title = String::from_utf16_lossy(&buffer[..length as usize]);
        if title != search.title {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return BOOL(1) };
        let mut image = [0u16; 1024];
        let mut size = image.len() as u32;
        let queried = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(image.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        if queried.is_ok() {
            let image = String::from_utf16_lossy(&image[..size as usize]);
            if super::is_ours(&title, search.title, &image, search.engine) {
                search.found = Some(window.0 as isize);
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    /// The window with this exact title that belongs to `engine`, if there is one.
    pub fn find_window(title: &str, engine: &Path) -> Option<isize> {
        let mut search = Search { title, engine, found: None };
        // EnumWindows reports an error when the callback stops it early; that is how it is told to.
        let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
        search.found
    }

    /// Takes the frame and caption off, removes the window from the taskbar, and puts it at the
    /// bottom of the stack beyond the right edge of every monitor, without activating it. It is
    /// never minimised: a minimised window stops being captured.
    pub fn make_background(handle: isize, width: u32, height: u32, x: i32) -> Result<(), String> {
        unsafe {
            let window = hwnd(handle);
            let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
            let style = (style & !(WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX)) | WS_POPUP;
            SetWindowLongPtrW(window, GWL_STYLE, style as isize);
            let extended = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
            let extended = (extended | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
            SetWindowLongPtrW(window, GWL_EXSTYLE, extended as isize);
            SetWindowPos(window, Some(HWND_BOTTOM), x, 0, width as i32, height as i32, SWP_FRAMECHANGED | SWP_NOACTIVATE).map_err(|e| e.to_string())
        }
    }

    pub fn client_size(handle: isize) -> Option<(u32, u32)> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(hwnd(handle), &mut rect).ok()? };
        Some((u32::try_from(rect.right - rect.left).ok()?, u32::try_from(rect.bottom - rect.top).ok()?))
    }

    /// Asks a window to close, the way its own close button does.
    pub fn post_close(handle: isize) {
        unsafe {
            let _ = PostMessageW(Some(hwnd(handle)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    /// Whether a program with this file name is running (for "Wallpaper Engine will start").
    pub fn process_running(exe_name: &str) -> bool {
        use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return false };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        unsafe {
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more {
                let length = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                if String::from_utf16_lossy(&entry.szExeFile[..length]).eq_ignore_ascii_case(exe_name) {
                    found = true;
                    break;
                }
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
        }
        found
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Display;
    use std::path::Path;

    pub fn foreground() -> Option<isize> {
        None
    }
    pub fn restore_foreground(_handle: isize) {}
    pub fn display() -> Option<Display> {
        None
    }
    pub fn find_window(_title: &str, _engine: &Path) -> Option<isize> {
        None
    }
    pub fn make_background(_handle: isize, _width: u32, _height: u32, _x: i32) -> Result<(), String> {
        Err("Recording needs Windows.".into())
    }
    pub fn client_size(_handle: isize) -> Option<(u32, u32)> {
        None
    }
    pub fn post_close(_handle: isize) {}
    pub fn process_running(_exe_name: &str) -> bool {
        false
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_compared_as_the_same_program_file() {
        let exe = r"C:\Program Files (x86)\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe";
        assert!(same_image(exe, exe));
        assert!(same_image(&exe.to_uppercase(), exe), "Windows paths ignore case");
        assert!(same_image(&exe.replace('\\', "/"), exe), "either separator");
        assert!(same_image(&format!(r"\\?\{exe}"), exe), "the extended-length prefix canonicalize adds");
        assert!(!same_image(r"C:\Windows\notepad.exe", exe));
        assert!(!same_image(r"C:\Program Files (x86)\Steam\steamapps\common\wallpaper_engine\wallpaper32.exe", exe));
        assert!(!same_image("", ""), "an empty image is no image");
        assert!(!same_image(&format!("{exe}.evil"), exe));
    }

    #[test]
    fn a_window_is_ours_only_by_exact_title_and_by_wallpaper_engines_own_program() {
        let engine = Path::new(r"C:\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe");
        let title = "Claws 0123456789abcdef0123456789abcdef";
        let image = r"C:\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe";
        assert!(is_ours(title, title, image, engine));
        assert!(!is_ours("Claws 0123456789abcdef0123456789abcdee", title, image, engine), "another title");
        assert!(!is_ours(&format!("{title} "), title, image, engine), "a title with anything added");
        assert!(!is_ours(&title.to_lowercase(), title, image, engine), "titles are exact");
        assert!(!is_ours(title, title, r"C:\Users\Someone\Downloads\notepad.exe", engine), "the right title, someone else's program");
        assert!(!is_ours(title, title, "", engine));
    }

    #[cfg(windows)]
    #[test]
    fn no_window_is_found_for_a_title_nobody_uses() {
        let engine = Path::new(r"C:\no such folder\wallpaper64.exe");
        assert_eq!(find_window("Claws no such window 7f3a9c", engine), None);
        assert!(display().is_some_and(|d| d.primary.0 > 0 && d.max_client.0 >= d.primary.0.min(1)));
        assert!(!process_running("claws-no-such-program.exe"));
    }
}
