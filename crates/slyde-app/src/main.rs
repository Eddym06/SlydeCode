//! SlydeCode launcher.
//!
//! `slyde`            start the studio (default)
//! `slyde app`        same, explicit
//! `slyde app --present`  open the presenter in kiosk mode
//! `slyde mcp`        run the MCP server over stdio
//! `slyde export`     export the deck (PDF/PNG)

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use slyde_core::{AspectRatio, PresentationState, TemplateType, Theme};
use slyde_server::chrome::OpenMode;
use slyde_server::{
    RuntimeInfo, ServerConfig, clear_runtime, discover_browser, probe_health, read_runtime,
    write_runtime,
};
use tokio::net::TcpListener;

#[derive(Debug, Default)]
struct AppArgs {
    present: bool,
    no_open: bool,
    ui: Option<PathBuf>,
    port: u16,
}

fn main() -> ExitCode {
    init_tracing();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("app");

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
        "app" | "studio" => runtime.block_on(run_app(&args[1..])),
        "mcp" => runtime.block_on(async {
            slyde_mcp::run_stdio().await.map_err(|err| err.to_string())
        }),
        "export" => runtime.block_on(async { slyde_export::run_cli(&args[1..]).await.map_err(|e| e.to_string()) }),
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

async fn run_app(raw_args: &[String]) -> Result<(), String> {
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
            open_window(&url, args.present, args.no_open)?;
            return Ok(());
        }
        clear_runtime();
    }

    let state = seed_state();
    let mut config = ServerConfig::new(ui_dir, state);
    config.port = args.port;

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
    open_window(&url, args.present, args.no_open)?;

    println!("Press Ctrl+C to quit.");

    let _ = tokio::signal::ctrl_c().await;

    server.shutdown().await;
    clear_runtime();
    println!("SlydeCode stopped.");
    Ok(())
}

fn open_window(url: &str, present: bool, no_open: bool) -> Result<(), String> {
    if no_open {
        println!("Open this URL in Chrome: {url}");
        return Ok(());
    }

    let browser = match discover_browser() {
        Ok(browser) => browser,
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
            println!("Could not launch {}. Open this URL manually: {url}", browser.path.display());
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
