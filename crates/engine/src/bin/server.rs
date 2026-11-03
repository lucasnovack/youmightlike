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
    about = "Servidor HTTP nativo em Rust (Axum) com grafo CSR persistente em RAM para o youmightlike"
)]
struct ServerArgs {
    /// Endereço IP para bind do servidor
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Porta HTTP do servidor
    #[arg(short, long, default_value_t = 8000)]
    port: u16,

    /// Caminho do banco DuckDB
    #[arg(short, long, default_value = "data/processed/taste_graph.duckdb")]
    db_path: String,

    /// Diretório de arquivos estáticos da interface web
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
