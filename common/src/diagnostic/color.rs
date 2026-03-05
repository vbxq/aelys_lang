use super::Severity;

#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub use_color: bool,
}

impl ColorConfig {
    pub fn auto() -> Self {
        let use_color = !std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty())
            && is_terminal_stderr();
        Self { use_color }
    }

    pub fn always() -> Self {
        Self { use_color: true }
    }

    pub fn never() -> Self {
        Self { use_color: false }
    }

    /// Bold + severity color (red for error, yellow for warning, cyan for help, default for note)
    pub fn severity(&self, severity: Severity, text: &str) -> String {
        if !self.use_color {
            return text.to_string();
        }
        let color_code = match severity {
            Severity::Error => "\x1b[1;31m",   // bold red
            Severity::Warning => "\x1b[1;33m", // bold yellow
            Severity::Help => "\x1b[1;36m",    // bold cyan
            Severity::Note => "\x1b[1m",       // bold (default color)
        };
        format!("{}{}\x1b[0m", color_code, text)
    }

    /// bold text (for error codes, etc.)
    pub fn bold(&self, text: &str) -> String {
        if !self.use_color {
            return text.to_string();
        }
        format!("\x1b[1m{}\x1b[0m", text)
    }

    /// blue (for line numbers and gutters)
    pub fn blue(&self, text: &str) -> String {
        if !self.use_color {
            return text.to_string();
        }
        format!("\x1b[1;34m{}\x1b[0m", text)
    }
}

fn is_terminal_stderr() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let handle = std::io::stderr().as_raw_handle();
        // if we can get console mode, it's a real console
        unsafe {
            let mut mode = 0u32;
            windows_sys_get_console_mode(handle, &mut mode)
        }
    }
    #[cfg(not(windows))]
    {
        // on unix, check if stderr is a tty
        false // conservative default but we could use libc::isatty
    }
}

#[cfg(windows)]
unsafe fn windows_sys_get_console_mode(handle: std::os::windows::io::RawHandle, mode: &mut u32) -> bool {
    unsafe extern "system" {
        fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
    }
    unsafe { GetConsoleMode(handle as *mut _, mode as *mut _) != 0 }
}
