use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tokio::net::TcpListener;
use youmightlike_engine::service::{build_router, load_app_state};

#[derive(Parser, Debug)]
#[command(
    name = "youmightlike-server",
    author = "Lucas Novack",
    version = "0.3.0",
    about = "Native Rust HTTP server (Axum) with persistent in-RAM CSR graph for youmightlike"
)]
struct ServerArgs {
    /// Server IP bind address
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Server HTTP port
    #[arg(short, long, default_value_t = 8000)]
    port: u16,

    /// DuckDB database path
    #[arg(short, long, default_value = "data/processed/taste_graph.duckdb")]
    db_path: String,

    /// Static web assets directory
    #[arg(long, default_value = "python/src/youmightlike/web/static")]
    static_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = ServerArgs::parse();

    let state = load_app_state(&args.db_path)?;
    let app = build_router(state, args.static_dir);

    let bind_addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&bind_addr).await?;
    println!("[youmightlike-server] listening on http://{}", bind_addr);

    axum::serve(listener, app).await?;
    Ok(())
}
