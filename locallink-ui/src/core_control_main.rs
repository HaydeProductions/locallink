#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(deprecated, dead_code)]

extern crate eframe as real_eframe;
extern crate std as real_std;

#[allow(dead_code)]
mod eframe {
    use crate::real_std::error::Error;
    use crate::real_std::sync::Arc;

    pub use crate::real_eframe::{App, CreationContext, Frame, NativeOptions, Result};

    pub type AppCreator = Box<
        dyn FnOnce(
            &CreationContext<'_>,
        ) -> crate::real_std::result::Result<
            Box<dyn App>,
            Box<dyn Error + Send + Sync>,
        > + 'static,
    >;

    pub fn run_native(
        app_name: &str,
        mut native_options: NativeOptions,
        app_creator: AppCreator,
    ) -> Result {
        native_options.viewport = native_options.viewport.with_icon(local_link_window_icon());
        crate::real_eframe::run_native(app_name, native_options, app_creator)
    }

    fn local_link_window_icon() -> Arc<egui::IconData> {
        let size = 64usize;
        let mut rgba = vec![0u8; size * size * 4];

        for y in 0..size {
            for x in 0..size {
                let dx = x.min(size - 1 - x) as f32;
                let dy = y.min(size - 1 - y) as f32;
                let radius = 13.0;
                let corner = if dx < radius && dy < radius {
                    let cx = radius - dx;
                    let cy = radius - dy;
                    (cx * cx + cy * cy).sqrt() <= radius
                } else {
                    true
                };

                if corner {
                    let i = (y * size + x) * 4;
                    let t = y as f32 / (size - 1) as f32;
                    rgba[i] = (12.0 + 8.0 * t) as u8;
                    rgba[i + 1] = (22.0 + 12.0 * t) as u8;
                    rgba[i + 2] = (44.0 + 26.0 * t) as u8;
                    rgba[i + 3] = 255;
                }
            }
        }

        draw_line(&mut rgba, size, 19.0, 33.0, 32.0, 20.0, [87, 232, 255, 255], 5.0);
        draw_line(&mut rgba, size, 32.0, 20.0, 45.0, 33.0, [87, 232, 255, 255], 5.0);
        draw_line(&mut rgba, size, 19.0, 33.0, 32.0, 44.0, [98, 255, 173, 255], 5.0);
        draw_line(&mut rgba, size, 32.0, 44.0, 45.0, 33.0, [98, 255, 173, 255], 5.0);

        draw_circle(&mut rgba, size, 19.0, 33.0, 7.0, [87, 232, 255, 255]);
        draw_circle(&mut rgba, size, 45.0, 33.0, 7.0, [98, 255, 173, 255]);
        draw_circle(&mut rgba, size, 32.0, 20.0, 5.5, [230, 255, 255, 255]);
        draw_circle(&mut rgba, size, 32.0, 44.0, 5.5, [230, 255, 255, 255]);
        draw_circle(&mut rgba, size, 19.0, 33.0, 3.0, [230, 255, 255, 255]);
        draw_circle(&mut rgba, size, 45.0, 33.0, 3.0, [230, 255, 255, 255]);
        draw_circle(&mut rgba, size, 32.0, 20.0, 2.2, [87, 232, 255, 255]);
        draw_circle(&mut rgba, size, 32.0, 44.0, 2.2, [98, 255, 173, 255]);

        Arc::new(egui::IconData {
            rgba,
            width: size as u32,
            height: size as u32,
        })
    }

    fn draw_circle(rgba: &mut [u8], size: usize, cx: f32, cy: f32, r: f32, color: [u8; 4]) {
        let min_x = (cx - r - 1.0).floor().max(0.0) as usize;
        let max_x = (cx + r + 1.0).ceil().min((size - 1) as f32) as usize;
        let min_y = (cy - r - 1.0).floor().max(0.0) as usize;
        let max_y = (cy + r + 1.0).ceil().min((size - 1) as f32) as usize;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                if dx * dx + dy * dy <= r * r {
                    blend_pixel(rgba, size, x, y, color);
                }
            }
        }
    }

    fn draw_line(
        rgba: &mut [u8],
        size: usize,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [u8; 4],
        width: f32,
    ) {
        let min_x = (x0.min(x1) - width).floor().max(0.0) as usize;
        let max_x = (x0.max(x1) + width).ceil().min((size - 1) as f32) as usize;
        let min_y = (y0.min(y1) - width).floor().max(0.0) as usize;
        let max_y = (y0.max(y1) + width).ceil().min((size - 1) as f32) as usize;
        let vx = x1 - x0;
        let vy = y1 - y0;
        let len2 = vx * vx + vy * vy;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32;
                let py = y as f32;
                let t = (((px - x0) * vx + (py - y0) * vy) / len2).clamp(0.0, 1.0);
                let cx = x0 + t * vx;
                let cy = y0 + t * vy;
                let dx = px - cx;
                let dy = py - cy;
                if dx * dx + dy * dy <= width * width {
                    blend_pixel(rgba, size, x, y, color);
                }
            }
        }
    }

    fn blend_pixel(rgba: &mut [u8], size: usize, x: usize, y: usize, color: [u8; 4]) {
        let i = (y * size + x) * 4;
        let a = color[3] as f32 / 255.0;
        let inv = 1.0 - a;
        rgba[i] = (color[0] as f32 * a + rgba[i] as f32 * inv) as u8;
        rgba[i + 1] = (color[1] as f32 * a + rgba[i + 1] as f32 * inv) as u8;
        rgba[i + 2] = (color[2] as f32 * a + rgba[i + 2] as f32 * inv) as u8;
        rgba[i + 3] = 255;
    }

    pub mod egui {
        pub use crate::real_eframe::egui::*;

        pub struct CentralPanel {
            inner: crate::real_eframe::egui::CentralPanel,
        }

        impl Default for CentralPanel {
            fn default() -> Self {
                Self {
                    inner: crate::real_eframe::egui::CentralPanel::default(),
                }
            }
        }

        impl CentralPanel {
            pub fn show<R>(
                self,
                ctx: &Context,
                add_contents: impl FnOnce(&mut Ui) -> R,
            ) -> InnerResponse<R> {
                self.inner.show(ctx, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Stop Core").clicked() {
                            crate::ui_shutdown_core();
                        }

                        if ui.button("Start Core").clicked() {
                            let _ = crate::ui_start_core_hidden();
                        }
                    });

                    ui.add_space(6.0);
                    add_contents(ui)
                })
            }
        }

        pub struct Window<'open> {
            inner: crate::real_eframe::egui::Window<'open>,
        }

        impl<'open> Window<'open> {
            pub fn new(title: impl Into<WidgetText>) -> Self {
                Self {
                    inner: crate::real_eframe::egui::Window::new(title),
                }
            }

            pub fn open(mut self, open: &'open mut bool) -> Self {
                self.inner = self.inner.open(open);
                self
            }

            pub fn default_width(mut self, width: f32) -> Self {
                self.inner = self.inner.default_width(width);
                self
            }

            pub fn default_height(mut self, height: f32) -> Self {
                self.inner = self.inner.default_height(height);
                self
            }

            pub fn resizable(mut self, resizable: bool) -> Self {
                self.inner = self.inner.resizable(resizable);
                self
            }

            pub fn show<R>(
                self,
                ctx: &Context,
                add_contents: impl FnOnce(&mut Ui) -> R,
            ) -> Option<InnerResponse<Option<R>>> {
                self.inner.show(ctx, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, add_contents)
                        .inner
                })
            }
        }
    }
}

fn ui_start_core_hidden() -> crate::real_std::io::Result<crate::real_std::process::Child> {
    let current = crate::real_std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| crate::real_std::io::Error::other("could not determine UI folder"))?;
    let core = dir.join("locallink-core.exe");

    let mut command = crate::real_std::process::Command::new(core);
    command
        .current_dir(dir)
        .stdin(crate::real_std::process::Stdio::null())
        .stdout(crate::real_std::process::Stdio::null())
        .stderr(crate::real_std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use crate::real_std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    command.spawn()
}

fn ui_shutdown_core() {
    let _ = ui_send_shutdown_api();

    #[cfg(target_os = "windows")]
    for image in ["locallink-addon-clipboard.exe", "locallink-core.exe"] {
        let mut command = crate::real_std::process::Command::new("taskkill.exe");
        command
            .args(["/F", "/T", "/IM", image])
            .stdin(crate::real_std::process::Stdio::null())
            .stdout(crate::real_std::process::Stdio::null())
            .stderr(crate::real_std::process::Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use crate::real_std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let _ = command.spawn().and_then(|mut child| child.wait());
    }
}

fn ui_send_shutdown_api() -> crate::real_std::io::Result<()> {
    use crate::real_std::io::Write;

    let mut stream = crate::real_std::net::TcpStream::connect("127.0.0.1:47900")?;
    stream.write_all(br#"{"cmd":"shutdown"}"#)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

#[allow(dead_code)]
mod std {
    pub use crate::real_std::{env, str, thread};

    pub mod collections {
        pub use crate::real_std::collections::*;
    }

    pub mod fs {
        pub use crate::real_std::fs::*;
    }

    pub mod io {
        pub use crate::real_std::io::*;
    }

    pub mod net {
        pub use crate::real_std::net::*;
    }

    pub mod path {
        pub use crate::real_std::path::*;
    }

    pub mod sync {
        pub use crate::real_std::sync::*;
    }

    pub mod time {
        pub use crate::real_std::time::*;
    }

    pub mod process {
        use crate::real_std::ffi::OsStr;
        use crate::real_std::io;
        #[cfg(target_os = "windows")]
        use crate::real_std::os::windows::process::CommandExt;
        use crate::real_std::path::Path;

        pub use crate::real_std::process::{Child, Stdio};

        pub struct Command(crate::real_std::process::Command);

        impl Command {
            pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
                let mut command = crate::real_std::process::Command::new(program);

                #[cfg(target_os = "windows")]
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW

                Self(command)
            }

            pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
                self.0.arg(arg);
                self
            }

            pub fn args<I, S>(&mut self, args: I) -> &mut Self
            where
                I: IntoIterator<Item = S>,
                S: AsRef<OsStr>,
            {
                self.0.args(args);
                self
            }

            pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
                self.0.current_dir(dir);
                self
            }

            pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
                self.0.stdin(cfg);
                self
            }

            pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
                self.0.stdout(cfg);
                self
            }

            pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
                self.0.stderr(cfg);
                self
            }

            pub fn spawn(&mut self) -> io::Result<Child> {
                self.0.spawn()
            }
        }
    }
}

include!("main.rs");
