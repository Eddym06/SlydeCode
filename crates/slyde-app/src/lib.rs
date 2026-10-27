//! SlydeCode launcher.
//!
//! Two executables share this crate:
//!
//! * `slyde` (console): the command line and the MCP server.
//!   `slyde` / `slyde app` start the studio, `slyde app --present` opens the
//!   presenter in kiosk mode, `slyde mcp` runs the MCP server over stdio and
//!   `slyde export` exports the deck.
//! * `slydecode` (no console on Windows): what a user double-clicks. It opens the
//!   studio and stops the server by itself when the last window is closed.

mod export_cli;
pub mod launcher;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use slyde_core::{AspectRatio, PresentationState, TemplateType, Theme};
use slyde_server::chrome::OpenMode;
use slyde_server::presence::{Abandoned, wait_until_abandoned};
use slyde_server::{
    RuntimeInfo, ServerConfig, clear_runtime, discover_browser, probe_health, read_runtime,
    write_runtime,
};
use tokio::net::TcpListener;

/// How long the browser may take to open the app before the launcher gives up.
/// A cold Chrome start on a slow disk can take several seconds.
const FIRST_WINDOW_TIMEOUT: Duration = Duration::from_secs(45);

/// How long every window must stay gone before the server stops. A reload, or a
/// switch between the editor and the presenter, drops the socket for a moment.
const CLOSE_GRACE: Duration = Duration::from_secs(8);

#[derive(Debug, Default)]
struct AppArgs {
    present: bool,
    no_open: bool,
    /// Windowless launcher: stop the server once the last window is closed, and
    /// treat a missing browser as an error instead of printing a URL nobody sees.
    exit_when_closed: bool,
    ui: Option<PathBuf>,
    port: u16,
}

/// Entry point of the console executable (`slyde`).
pub fn run_cli() -> ExitCode {
    init_tracing();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = split_command(&args);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build();

    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("slyde: could not start async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        "app" | "studio" => runtime.block_on(run_app(rest)),
        "mcp" => {
            runtime.block_on(async { slyde_mcp::run_stdio().await.map_err(|err| err.to_string()) })
        }
        "export" => runtime.block_on(export_cli::run(rest)),
        "version" | "--version" | "-V" => {
            println!("slyde {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}. Try `slyde help`.")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("slyde: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Splits the command line into the command and its own arguments.
///
/// A bare launch (double-clicking `slyde.exe`) has no arguments, and
/// `slyde --no-open` starts with an option: both mean "open the studio", as the
/// help text promises. Only `--version`/`-V` and `--help`/`-h` stay commands.
fn split_command(args: &[String]) -> (&str, &[String]) {
    match args.first().map(String::as_str) {
        None => ("app", args),
        Some(first @ ("--version" | "-V" | "--help" | "-h")) => (first, &args[1..]),
        Some(first) if first.starts_with('-') => ("app", args),
        Some(first) => (first, &args[1..]),
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Always stderr: stdout belongs to the MCP protocol.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

fn print_help() {
    println!(
        "slyde {version}\n\
         \n\
         Usage:\n\
           slyde [app] [options]     open the studio\n\
           slyde app --present       open the presenter in kiosk mode\n\
           slyde mcp                 run the MCP server over stdio\n\
           slyde export [options]    export the deck\n\
         \n\
         Options:\n\
           --no-open                 do not launch a browser\n\
           --ui <dir>                serve the UI from <dir>\n\
           --port <n>                bind a specific port (default: ephemeral)\n",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn parse_app_args(args: &[String]) -> Result<AppArgs, String> {
    let mut parsed = AppArgs::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--present" => parsed.present = true,
            "--no-open" => parsed.no_open = true,
            "--exit-when-closed" => parsed.exit_when_closed = true,
            "--ui" => {
                index += 1;
                let value = args.get(index).ok_or("--ui needs a directory")?;
                parsed.ui = Some(PathBuf::from(value));
            }
            "--port" => {
                index += 1;
                let value = args.get(index).ok_or("--port needs a number")?;
                parsed.port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            other => return Err(format!("unknown option: {other}")),
        }
        index += 1;
    }
    Ok(parsed)
}

pub(crate) async fn run_app(raw_args: &[String]) -> Result<(), String> {
    let args = parse_app_args(raw_args)?;
    let ui_dir = resolve_ui_dir(args.ui);

    // Already running? Hand the URL to a new window and exit.
    if let Some(info) = read_runtime() {
        if probe_health(info.port, Duration::from_millis(500)).await {
            let url = if args.present {
                info.present_url()
            } else {
                info.url()
            };
            open_window(&url, args.present, args.no_open, args.exit_when_closed)?;
            return Ok(());
        }
        clear_runtime();
    }

    // The server owns the deck library: it opens the active deck (or seeds one)
    // and autosaves every mutation to that file. This state is only the fallback
    // for a memory-only server.
    let state = seed_state();
    let mut config = ServerConfig::new(ui_dir, state);
    config.port = args.port;
    config.assets_dir = Some(resolve_assets_dir());
    config.library_dir = Some(slyde_server::storage::decks_dir());

    let server = slyde_server::start(config)
        .await
        .map_err(|err| err.to_string())?;

    let info = server.runtime_info();
    write_runtime(&info).map_err(|err| format!("could not write runtime file: {err}"))?;

    println!("SlydeCode is running at {}", server.app_url());

    let url = if args.present {
        server.present_url()
    } else {
        server.app_url()
    };
    open_window(&url, args.present, args.no_open, args.exit_when_closed)?;

    println!("Press Ctrl+C to quit.");

    // Ctrl+C always stops the server. The windowless launcher also stops it when
    // the app was never opened or its last window has been closed.
    let ended = tokio::select! {
        _ = tokio::signal::ctrl_c() => None,
        outcome = wait_for_windows(&server, args.exit_when_closed) => Some(outcome),
    };

    // Flush before exiting: the debounced autosave may be waiting on the last
    // change, and Ctrl+C would otherwise drop it.
    if let Some(library) = &server.library
        && let Some(active) = library.active()
    {
        let deck = server.state.core.read().await.presentation().clone();
        if let Err(err) = slyde_server::storage::save(&library.path_for(&active), &deck).await {
            eprintln!("slyde: could not save the deck on exit: {err}");
        }
    }

    server.shutdown().await;
    clear_runtime();
    match ended {
        Some(Abandoned::Closed) => tracing::info!("every window was closed; server stopped"),
        Some(Abandoned::NeverOpened) => tracing::warn!("no window connected; server stopped"),
        None => tracing::info!("stopped by Ctrl+C"),
    }
    println!("SlydeCode stopped.");
    match ended {
        Some(Abandoned::NeverOpened) => Err(
            "The SlydeCode window never opened, so the server was stopped. \
             Check that Chrome, Edge, Chromium or Brave starts, then open SlydeCode again."
                .to_string(),
        ),
        _ => Ok(()),
    }
}

/// Resolves when the launcher should stop the server; never resolves for the
/// console build, which keeps running until Ctrl+C.
async fn wait_for_windows(server: &slyde_server::RunningServer, enabled: bool) -> Abandoned {
    if !enabled {
        return std::future::pending().await;
    }
    wait_until_abandoned(
        server.state.presence.subscribe(),
        FIRST_WINDOW_TIMEOUT,
        CLOSE_GRACE,
    )
    .await
}

fn open_window(url: &str, present: bool, no_open: bool, strict: bool) -> Result<(), String> {
    if no_open {
        println!("Open this URL in Chrome: {url}");
        return Ok(());
    }

    let browser = match discover_browser() {
        Ok(browser) => browser,
        Err(_) if strict => {
            return Err("No Chrome, Edge, Chromium or Brave was found. \
                 Install one of them and open SlydeCode again."
                .to_string());
        }
        Err(_) => {
            println!("No Chrome/Chromium/Edge found. Open this URL manually: {url}");
            return Ok(());
        }
    };

    let profile = slyde_server::app_data_dir().join("chrome-profile");
    let mode = if present {
        OpenMode::Kiosk
    } else {
        OpenMode::App
    };

    match browser.launch(url, mode, &profile) {
        Ok(_child) => Ok(()),
        Err(err) => {
            println!(
                "Could not launch {}. Open this URL manually: {url}",
                browser.path.display()
            );
            Err(format!("browser launch failed: {err}"))
        }
    }
}

/// First-run deck: one on-brand hero slide so the studio is never an empty void.
fn seed_state() -> PresentationState {
    let mut state = PresentationState::new("Untitled deck", Theme::studio(), AspectRatio::R16x9);
    let _ = state.upsert_slide(
        0,
        None,
        slyde_server::starter::default_code("hero"),
        TemplateType::Hero,
        slyde_server::starter::default_animation_for("hero"),
    );
    state
}

fn resolve_ui_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    if let Ok(from_env) = std::env::var("SLYDE_UI_DIR") {
        return PathBuf::from(from_env);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("ui"));
            candidates.push(dir.join("../../../ui"));
        }
    }
    candidates.push(PathBuf::from("ui"));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui"));

    candidates
        .iter()
        .find(|path| path.join("index.html").is_file())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("ui"))
}

/// Fonts and brand assets. Same resolution rules as the UI: explicit env,
/// then relative to the executable (bundled build), then the repo checkout.
fn resolve_assets_dir() -> PathBuf {
    if let Ok(from_env) = std::env::var("SLYDE_ASSETS_DIR") {
        return PathBuf::from(from_env);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("assets"));
            candidates.push(dir.join("../../../assets"));
        }
    }
    candidates.push(PathBuf::from("assets"));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));

    candidates
        .iter()
        .find(|path| path.join("fonts").is_dir())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("assets"))
}

/// Kept here so `slyde mcp` and the studio agree on the runtime file location.
pub fn runtime_info_for(port: u16, sandbox_port: u16, token: String) -> RuntimeInfo {
    RuntimeInfo {
        port,
        sandbox_port,
        token,
        pid: std::process::id(),
        started_at_ms: slyde_server::now_ms(),
    }
}

/// Reserved for the `--port` flow: verifies a port is free before starting.
pub async fn port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::split_command;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn a_bare_launch_opens_the_studio() {
        let args = words(&[]);
        assert_eq!(split_command(&args), ("app", &args[..]));
    }

    #[test]
    fn an_option_without_a_command_opens_the_studio_with_that_option() {
        let args = words(&["--no-open", "--port", "0"]);
        let (command, rest) = split_command(&args);
        assert_eq!(command, "app");
        assert_eq!(rest, &args[..]);
    }

    #[test]
    fn an_explicit_command_keeps_its_own_arguments() {
        let args = words(&["export", "--format", "pdf"]);
        let (command, rest) = split_command(&args);
        assert_eq!(command, "export");
        assert_eq!(rest, &args[1..]);
        let args = words(&["app", "--present"]);
        assert_eq!(split_command(&args), ("app", &args[1..]));
    }

    #[test]
    fn version_and_help_flags_stay_commands() {
        for flag in ["--version", "-V", "--help", "-h"] {
            let args = words(&[flag]);
            assert_eq!(split_command(&args).0, flag);
        }
    }
}
