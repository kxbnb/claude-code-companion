mod app;
mod process;
mod protocol;
mod server;
mod ui;

use clap::Parser;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "companion-tui",
    about = "Terminal companion for Claude Code",
    version
)]
struct Args {
    /// Port for the WebSocket server
    #[arg(long, default_value = "8765")]
    port: u16,

    /// Working directory for Claude Code
    #[arg(long)]
    cwd: Option<String>,

    /// Model to use (e.g., claude-sonnet-4-5-20250929)
    #[arg(long)]
    model: Option<String>,

    /// Don't spawn a CLI process (connect to existing)
    #[arg(long)]
    connect: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // ── Logging (file-based, since stdout is the TUI) ────────────────────
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("companion-tui");
    std::fs::create_dir_all(&log_dir)?;
    let file_appender = tracing_appender::rolling::daily(&log_dir, "companion-tui.log");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("claude_code_companion=debug")),
        )
        .with_writer(file_appender)
        .with_ansi(false)
        .init();

    tracing::info!("Starting companion-tui on port {}", args.port);

    // ── Event channel ────────────────────────────────────────────────────
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // ── Resolve working directory ────────────────────────────────────────
    let cwd = args.cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string()
    });

    tracing::info!("Working directory: {}", cwd);

    // ── App state ────────────────────────────────────────────────────────
    let mut app = app::App::new(args.port, cwd.clone(), args.model.clone());

    // Load environment profiles from ~/.companion/envs/
    app.load_env_profiles();
    if !app.env_profiles.is_empty() {
        tracing::info!("Loaded {} environment profiles", app.env_profiles.len());
    }

    // Load persisted sessions from ~/.companion/sessions/
    app.load_persisted_sessions();
    if !app.session_order.is_empty() {
        tracing::info!(
            "Loaded {} persisted sessions",
            app.session_order.len()
        );
    }

    // ── WebSocket server ─────────────────────────────────────────────────
    let ws_server = server::ws_server::WsServer::bind(args.port, event_tx.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind WebSocket port {}: {} (is another companion running?)", args.port, e))?;
    tokio::spawn(async move {
        if let Err(e) = ws_server.run().await {
            tracing::error!("WebSocket server error: {}", e);
        }
    });

    // ── Create initial session if none loaded ────────────────────────────
    if app.session_order.is_empty() {
        let name = app::generate_session_name();
        tracing::info!("Creating initial session: {}", name);

        if args.connect {
            // --connect mode: create session but don't spawn CLI
            let id = uuid::Uuid::new_v4().to_string();
            let session = app::Session::new(id.clone(), name, cwd);
            app.sessions.insert(id.clone(), session);
            app.session_order.push(id.clone());
            app.active_session_id = Some(id);
        } else {
            // Normal mode: create session and queue CLI spawn
            app.create_session(name, cwd, None);
        }
    }

    // ── Run the TUI event loop (blocks until quit) ───────────────────────
    ui::event_loop::run(app, event_rx, event_tx).await?;

    tracing::info!("companion-tui exiting");
    Ok(())
}
