#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(deprecated, dead_code)]

extern crate std as real_std;

#[allow(dead_code)]
mod std {
    pub use crate::real_std::{env, thread};

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
