use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/main.rs");

    let source = fs::read_to_string("src/main.rs")
        .expect("read src/main.rs")
        .replace("\r\n", "\n");
    let mut generated = source;

    generated = must_replace(
        generated,
        "fn main() -> eframe::Result {\n    let options = eframe::NativeOptions {",
        "fn main() -> eframe::Result {\n    start_windows_tray();\n\n    let options = eframe::NativeOptions {",
    );

    generated = must_replace(
        generated,
        "    fn start_core(&mut self) {\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n",
        "    fn start_core(&mut self) {\n        match start_sibling_core() {\n            Ok(()) => {\n                self.log(\"Starting LocalLink Core...\");\n                std::thread::sleep(Duration::from_millis(250));\n                self.refresh_all();\n            }\n            Err(e) => self.log(format!(\"Could not start core: {e}\")),\n        }\n    }\n\n    fn stop_core(&mut self) {\n        for (_, mut child) in self.addon_processes.drain() {\n            let _ = child.kill();\n            let _ = child.wait();\n        }\n\n        self.send_job(ApiJob::Shutdown);\n\n        self.status = None;\n        self.peers.clear();\n        self.connections.clear();\n        self.addons.clear();\n\n        self.log(\"Stopping LocalLink Core...\");\n    }\n\n",
    );

    generated = must_replace(
        generated,
        "                    if !self.core_online()\n                        && ui\n                            .add(primary_button(\"Start\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                    {\n                        self.start_core();\n                    }\n",
        "                    if self.core_online() {\n                        if ui\n                            .add(danger_button(\"Stop Core\"))\n                            .on_hover_cursor(egui::CursorIcon::PointingHand)\n                            .clicked()\n                        {\n                            self.stop_core();\n                        }\n                    } else if ui\n                        .add(primary_button(\"Start\"))\n                        .on_hover_cursor(egui::CursorIcon::PointingHand)\n                        .clicked()\n                    {\n                        self.start_core();\n                    }\n",
    );

    generated = generated.replace("secondary_button(\"Shutdown\")", "danger_button(\"Stop Core\")");
    generated.push_str(TRAY_CODE);

    fs::write(Path::new("src/core_control_main.rs"), generated)
        .expect("write generated UI entry point");
}

fn must_replace(input: String, from: &str, to: &str) -> String {
    let output = input.replace(from, to);

    if output == input {
        panic!("expected UI source pattern was not found while generating core-control entry point");
    }

    output
}

const TRAY_CODE: &str = r#"

#[cfg(not(target_os = "windows"))]
fn start_windows_tray() {}

#[cfg(target_os = "windows")]
fn start_windows_tray() {
    std::thread::spawn(|| unsafe {
        windows_tray_thread();
    });
}

#[cfg(target_os = "windows")]
unsafe fn windows_tray_thread() {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
        NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, CS_HREDRAW,
        CS_VREDRAW, CW_USEDEFAULT, HMENU, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
        TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_DESTROY,
        WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
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
                                show_ui_window();
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
                        show_ui_window();
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
        0,
        0,
        instance,
        null_mut::<c_void>(),
    );

    if hwnd == 0 {
        return;
    }

    add_tray_icon(hwnd);

    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    fn add_tray_icon(hwnd: HWND) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = LoadIconW(0, IDI_APPLICATION);
            write_wide_fixed(&mut nid.szTip, "LocalLink");

            Shell_NotifyIconW(NIM_ADD, &mut nid);
            nid.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &mut nid);
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
            if menu == 0 {
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

    fn show_ui_window() {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                FindWindowW, IsIconic, SetWindowPos, ShowWindow, SW_RESTORE, SW_SHOW,
                SWP_NOMOVE, SWP_NOSIZE, HWND_TOP,
            };

            let title = wide_null("LocalLink");
            let hwnd = FindWindowW(null(), title.as_ptr());
            if hwnd != 0 {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                } else {
                    ShowWindow(hwnd, SW_SHOW);
                }

                SetForegroundWindow(hwnd);
                SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            }
        }
    }

    fn shutdown_core_via_api() -> Result<()> {
        api_request(json!({ "cmd": "shutdown" })).map(|_| ())
    }

    fn kill_local_processes_for_exit() {
        let names = [
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
}
"#;
