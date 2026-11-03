use anyhow::Result;
use clap::Parser;
use std::time::Instant;
use youmightlike_engine::{run_personalized_pagerank_filtered, GraphCSR};

#[derive(Parser, Debug)]
#[command(
    name = "youmightlike-engine",
    author = "Lucas Novack",
    version = "0.3.0",
    about = "Core de difusão de grafos em alta performance para o youmightlike"
)]
struct Args {
    /// Caminho do banco DuckDB
    #[arg(short, long, default_value = "data/processed/taste_graph.duckdb")]
    db_path: String,

    /// Nós semente separados por vírgula (ex: "artist:11,movie:12406")
    #[arg(short, long, required = true)]
    seeds: String,

    /// Filtrar tipo de nó recomendado (ex: "track:", "artist:", "movie:", "game:", "book:")
    #[arg(short, long, default_value = "")]
    target_prefix: String,

    /// Quantidade de recomendações a retornar
    #[arg(short, long, default_value_t = 10)]
    limit: usize,

    /// Fator de teletransporte do PageRank (0.0 a 1.0)
    #[arg(long, default_value_t = 0.85)]
    alpha: f32,

    /// Número máximo de iterações do PPR
    #[arg(long, default_value_t = 25)]
    max_iter: usize,

    /// Tolerância de convergência (L1 norm)
    #[arg(long, default_value_t = 1e-6)]
    tol: f32,

    /// Formatar saída diretamente em JSON
    #[arg(long, default_value_t = false)]
    json: bool,

    /// Consulta SQL customizada para carregar arestas (ex: para subgrafos de teste rápido)
    #[arg(long)]
    query_edges: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let seed_keys: Vec<&str> = args.seeds.split(',').map(|s| s.trim()).collect();

    if !args.json {
        println!("[youmightlike-engine] graph diffusion core");
        println!("  database: {}", args.db_path);
        println!("  seeds: {:?}", seed_keys);
    }

    // 1. Carrega o Grafo do DuckDB para CSR em memória
    let t_load = Instant::now();
    if !args.json {
        print!("  [1/2] loading edges and constructing CSC sparse matrix... ");
    }
    let graph = GraphCSR::load_from_duckdb(&args.db_path, args.query_edges.as_deref())?;
    let load_duration = t_load.elapsed();

    if !args.json {
        println!(
            "done ({:.3}s, {} nodes, {} edges)",
            load_duration.as_secs_f32(),
            graph.num_nodes,
            graph.num_edges
        );
    }

    // 2. Executa o Personalized PageRank paralelo com Rayon e seleção top-K parcial
    let t_ppr = Instant::now();
    let target_pfx = if args.target_prefix.is_empty() {
        None
    } else {
        Some(args.target_prefix.as_str())
    };
    let filtered = run_personalized_pagerank_filtered(
        &graph,
        &seed_keys,
        args.alpha,
        args.max_iter,
        args.tol,
        target_pfx,
        Some(args.limit),
    )?;
    let ppr_duration = t_ppr.elapsed();

    if !args.json {
        println!(
            "  [2/2] ppr diffusion converged in {:.4}s",
            ppr_duration.as_secs_f32()
        );
    }

    if args.json {
        let json_output = serde_json::to_string_pretty(&filtered)?;
        println!("{}", json_output);
    } else {
        println!(
            "\nTop {} recommendations (structural diffusion):",
            filtered.len()
        );
        for (rank, item) in filtered.iter().enumerate() {
            println!(
                "  {:>2}. {:<25} (ppr score: {:.6})",
                rank + 1,
                item.node_key,
                item.score
            );
        }
        println!();
    }

    Ok(())
}
