#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::{bail, Context, Result};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const LOCAL_API_ADDR: &str = "127.0.0.1:47900";
const TRAY_ICON_B64: &str = include_str!("../../assets/locallink-tray.ico.b64");

#[cfg(not(target_os = "windows"))]
fn main() -> Result<()> {
    bail!("LocalLink tray is only supported on Windows")
}

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    unsafe { windows_tray_main() }
}

#[cfg(target_os = "windows")]
unsafe fn windows_tray_main() -> Result<()> {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, LoadImageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, CS_HREDRAW,
        CS_VREDRAW, CW_USEDEFAULT, HMENU, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE,
        LR_LOADFROMFILE, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
        WS_OVERLAPPED,
    };

    const TRAY_UID: u32 = 1;
    const WM_TRAYICON: u32 = WM_APP + 1;
    const MENU_OPEN: usize = 1001;
    const MENU_EXIT: usize = 1002;

    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_TRAYICON => {
                    if wparam as u32 == TRAY_UID {
                        match lparam as u32 {
                            WM_LBUTTONUP => {
                                let _ = launch_or_focus_ui();
                                return 0;
                            }
                            WM_RBUTTONUP => {
                                show_tray_menu(hwnd);
                                return 0;
                            }
                            _ => {}
                        }
                    }
                }
                WM_COMMAND => match (wparam & 0xffff) as usize {
                    MENU_OPEN => {
                        let _ = launch_or_focus_ui();
                        return 0;
                    }
                    MENU_EXIT => {
                        let _ = shutdown_core_via_api();
                        kill_local_processes_for_exit();
                        remove_tray_icon(hwnd);
                        PostQuitMessage(0);
                        return 0;
                    }
                    _ => {}
                },
                WM_DESTROY => {
                    remove_tray_icon(hwnd);
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    fn add_tray_icon(hwnd: HWND) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon =
                load_locallink_icon().unwrap_or_else(|| LoadIconW(null_mut(), IDI_APPLICATION));
            write_wide_fixed(&mut nid.szTip, "LocalLink");

            Shell_NotifyIconW(NIM_ADD, &mut nid);
        }
    }

    fn remove_tray_icon(hwnd: HWND) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            Shell_NotifyIconW(NIM_DELETE, &mut nid);
        }
    }

    fn show_tray_menu(hwnd: HWND) {
        unsafe {
            let menu: HMENU = CreatePopupMenu();
            if menu.is_null() {
                return;
            }

            let open = wide_null("Open LocalLink");
            let exit = wide_null("Exit");
            AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());

            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            SetForegroundWindow(hwnd);
            TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                null(),
            );
            DestroyMenu(menu);
        }
    }

    fn load_locallink_icon() -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
        let path = write_tray_icon_file().ok()?;
        let wide = wide_null(&path.display().to_string());

        let handle = unsafe {
            LoadImageW(
                null_mut(),
                wide.as_ptr(),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
        };

        if handle.is_null() {
            None
        } else {
            Some(handle)
        }
    }

    fn wide_null(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn write_wide_fixed<const N: usize>(dest: &mut [u16; N], text: &str) {
        let wide = wide_null(text);
        let copy_len = wide.len().min(N);
        dest[..copy_len].copy_from_slice(&wide[..copy_len]);
        if copy_len == N {
            dest[N - 1] = 0;
        }
    }

    let instance = GetModuleHandleW(null());
    let class_name = wide_null("LocalLinkTrayWindow");

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..zeroed()
    };

    RegisterClassW(&wnd_class);

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide_null("LocalLink Tray").as_ptr(),
        WS_OVERLAPPED,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        null_mut(),
        null_mut(),
        instance,
        null_mut::<c_void>(),
    );

    if hwnd.is_null() {
        bail!("could not create tray window");
    }

    add_tray_icon(hwnd);

    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn write_tray_icon_file() -> Result<PathBuf> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let appdata = std::env::var("APPDATA").context("APPDATA environment variable not found")?;
    let dir = PathBuf::from(appdata).join("LocalLink").join("assets");
    fs::create_dir_all(&dir)?;

    let icon_path = dir.join("locallink-tray.ico");
    let icon_bytes = STANDARD.decode(TRAY_ICON_B64.trim())?;
    fs::write(&icon_path, icon_bytes)?;

    Ok(icon_path)
}

#[cfg(target_os = "windows")]
fn launch_or_focus_ui() -> Result<()> {
    if focus_existing_ui_window() {
        return Ok(());
    }

    let ui = sibling_exe("LocalLink.exe")?;
    let dir = ui
        .parent()
        .ok_or_else(|| anyhow::anyhow!("could not determine LocalLink folder"))?;

    Command::new(&ui)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("launching LocalLink UI")?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn focus_existing_ui_window() -> bool {
    unsafe {
        use std::ptr::null;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, IsIconic, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP,
            SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, SW_SHOW,
        };

        let title = "LocalLink"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let hwnd = FindWindowW(null(), title.as_ptr());

        if hwnd.is_null() {
            return false;
        }

        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else {
            ShowWindow(hwnd, SW_SHOW);
        }

        SetForegroundWindow(hwnd);
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        true
    }
}

#[cfg(target_os = "windows")]
fn sibling_exe(name: &str) -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("could not determine tray executable folder"))?;
    let exe = dir.join(name);

    if !exe.exists() {
        bail!("{} not found next to tray executable", exe.display());
    }

    Ok(exe)
}

#[cfg(target_os = "windows")]
fn shutdown_core_via_api() -> Result<()> {
    api_request(json!({ "cmd": "shutdown" })).map(|_| ())
}

#[cfg(target_os = "windows")]
fn api_request(req: serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = TcpStream::connect(LOCAL_API_ADDR)
        .with_context(|| format!("could not connect to LocalLink Core API at {LOCAL_API_ADDR}"))?;

    let line = serde_json::to_string(&req)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    if response.trim().is_empty() {
        bail!("empty response from LocalLink Core API");
    }

    Ok(serde_json::from_str(&response)?)
}

#[cfg(target_os = "windows")]
fn kill_local_processes_for_exit() {
    let names = [
        "LocalLink.exe",
        "locallink-addon-clipboard.exe",
        "locallink-core.exe",
    ];

    for name in names {
        let _ = Command::new("taskkill.exe")
            .args(["/F", "/T", "/IM", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut child| child.wait());
    }
}
