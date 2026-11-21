//! Arbor Studio — a procedural tree modeller with a built-in MCP server.
//!
//! Usage:
//!   arbor_studio [project.arbor.json]       open the window
//!   arbor_studio --port 7878                 choose the MCP HTTP port (default 7878, env ARBOR_MCP_PORT)
//!   arbor_studio --no-mcp                    do not start the MCP server
//!   arbor_studio --mcp-stdio                 run as a stdio MCP server (relays to the window, starting it if needed)
//!   arbor_studio --headless [--mcp-stdio]    no window: MCP over HTTP, or in-process over stdio
//!   arbor_studio --screenshot out.png        draw ~120 frames, save the window as PNG and exit

mod app;
mod billboard;
mod blendbake;
mod collision;
mod docshots;
mod export;
mod gpu;
mod grow;
mod growth;
mod budget;
mod help;
mod leafarea;
mod leafmesh;
mod mcp;
#[cfg(target_os = "macos")]
mod native_menu;
mod recent;
mod model;
mod render;
mod state;
mod textures;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut screenshot: Option<PathBuf> = None;
    let mut port: u16 = std::env::var("ARBOR_MCP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(mcp::DEFAULT_PORT);
    let mut stdio = false;
    let mut headless = false;
    let mut mcp_enabled = true;
    let mut open: Option<PathBuf> = None;
    let mut shot_script: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--shot-script" => shot_script = args.next().map(PathBuf::from),
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--mcp-stdio" => stdio = true,
            "--no-mcp" => mcp_enabled = false,
            "--headless" => headless = true,
            "-h" | "--help" => {
                println!("ArborStudio {}\n\n  [project.arbor.json]   open a project\n  --port N               MCP HTTP port (default 7878)\n  --no-mcp               don't start the MCP server\n  --mcp-stdio            stdio MCP server relaying to the window (starts it if needed)\n  --headless             no window: with --mcp-stdio answer in-process, otherwise serve MCP over HTTP\n  --screenshot FILE.png  save a picture of the window and exit\n  --shot-script FILE     take the documentation's window pictures (see src/docshots.rs) and exit", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other if !other.starts_with("--") => open = Some(PathBuf::from(other)),
            _ => {}
        }
    }
    if stdio && !headless {
        mcp::run_stdio_bridge(port);
        return Ok(());
    }

    let mut doc = model::Document::default();
    let mut opened = None;
    if let Some(p) = &open {
        match state::load_project(p) {
            Ok(d) => {
                doc = d;
                opened = Some(p.clone());
            }
            Err(e) => eprintln!("{e}"),
        }
    }
    let mut shared = state::Shared::new(doc);
    shared.file = opened;
    shared.log(state::Source::App, "ArborStudio started");
    let shared = Arc::new(Mutex::new(shared));

    if headless {
        let server = mcp::Server { shared, ctx: egui::Context::default(), headless: true };
        if stdio {
            mcp::run_stdio_inprocess(server);
        } else {
            match mcp::start(server, port) {
                Ok(url) => {
                    eprintln!("ArborStudio (no window) — MCP server at {url}");
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
                Err(e) => eprintln!("MCP server could not start: {e}"),
            }
        }
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("ArborStudio")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 600.0]);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let mut options = eframe::NativeOptions::default();
    let quiet = screenshot.is_some() || shot_script.is_some();
    if quiet {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
        #[cfg(target_os = "macos")]
        {
            options.event_loop_builder = Some(Box::new(|b| {
                use winit::platform::macos::EventLoopBuilderExtMacOS;
                b.with_activate_ignoring_other_apps(false);
            }));
        }
    }
    options.viewport = viewport;
    options.run_and_return = false;
    // a depth buffer for the 3D viewport (and smoother edges)
    options.depth_buffer = 24;
    options.multisampling = 4;

    eframe::run_native(
        "Arbor Studio",
        options,
        Box::new(move |cc| {
            let mut a = app::ArborApp::new(cc, shared, port, screenshot, mcp_enabled || shot_script.is_some());
            if let Some(p) = &shot_script {
                match docshots::ShotScript::load(p) {
                    Ok(s) => a.script = Some(s),
                    Err(e) => eprintln!("{e}"),
                }
            }
            Ok(Box::new(a))
        }),
    )
}

/// The app's icon for the window, taskbar and (when not started from the app bundle) the Dock.
fn app_icon() -> Option<egui::IconData> {
    // macOS icons carry their own margin and shadow; elsewhere the icon fills its square
    #[cfg(target_os = "macos")]
    let bytes: &[u8] = include_bytes!("../assets/icon-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let bytes: &[u8] = include_bytes!("../assets/icon-256.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}
