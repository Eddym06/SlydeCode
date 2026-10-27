//! Entry point of the windowless executable (`slydecode`).
//!
//! A double-clicked program has no terminal to print to, so this module does the
//! three things a console would have done: it keeps a log file, it shows a dialog
//! when the app cannot start, and it lets the server stop on its own once the last
//! window is closed (see `slyde_server::presence`).

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;

/// The log is appended to across launches and restarted when it grows past this.
const MAX_LOG_BYTES: u64 = 512 * 1024;

/// Runs the studio for a user who double-clicked the executable.
pub fn run() -> ExitCode {
    init_file_logging();
    install_panic_report();

    let mut args = vec!["--exit-when-closed".to_string()];
    // `slydecode app --present` and `slydecode --present` mean the same thing.
    let mut passed = std::env::args().skip(1).peekable();
    if matches!(passed.peek().map(String::as_str), Some("app" | "studio")) {
        passed.next();
    }
    args.extend(passed);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return fail(&format!("Could not start the async runtime: {err}")),
    };

    match runtime.block_on(crate::run_app(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}

fn fail(message: &str) -> ExitCode {
    tracing::error!("{message}");
    dialog::error("SlydeCode", message);
    ExitCode::FAILURE
}

fn log_path() -> PathBuf {
    slyde_server::app_data_dir().join("slydecode.log")
}

fn init_file_logging() {
    use tracing_subscriber::EnvFilter;

    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let oversized = std::fs::metadata(&path)
        .map(|meta| meta.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    let mut options = OpenOptions::new();
    options.create(true);
    if oversized {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    // Without a log file there is nowhere to write; the app still runs.
    let Ok(file) = options.open(&path) else {
        return;
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .try_init();
}

/// A panic in a windowless process would vanish without a trace.
fn install_panic_report() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!(
            "SlydeCode stopped unexpectedly: {info}\n\nDetails were saved to {}",
            log_path().display()
        );
        tracing::error!("{message}");
        dialog::error("SlydeCode", &message);
    }));
}

#[cfg(windows)]
mod dialog {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MessageBoxW(owner: *mut c_void, text: *const u16, caption: *const u16, kind: u32)
        -> i32;
    }

    /// `MB_OK | MB_ICONERROR`.
    const ERROR_BOX: u32 = 0x10;

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    /// Shows a modal error box; the only way a windowless program can say
    /// "I could not start".
    pub fn error(title: &str, message: &str) {
        let text = wide(message);
        let caption = wide(title);
        // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call,
        // and a null owner window is the documented "no owner" value.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                ERROR_BOX,
            );
        }
    }
}

#[cfg(not(windows))]
mod dialog {
    pub fn error(title: &str, message: &str) {
        eprintln!("{title}: {message}");
    }
}
