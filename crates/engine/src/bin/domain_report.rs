//! Domain balance report: measures how PPR probability mass is distributed across media domains
//! for a fixed set of seed profiles, under several inter-domain balancing exponents (gamma).
//!
//! Usage:
//!   cargo run --release --bin domain-report -- --gammas 1.0,0.75,0.5,0.25,0.0 [--json out.json]

use anyhow::Result;
use clap::Parser;
use duckdb::{AccessMode, Config, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use youmightlike_engine::graph::{item_domain_of, RawEdges, ITEM_DOMAINS};
use youmightlike_engine::{run_personalized_pagerank_scores, GraphCSR};

#[derive(Parser, Debug)]
#[command(name = "domain-report", about = "Inter-domain PPR mass report")]
struct Args {
    #[arg(short, long, default_value = "data/processed/taste_graph.duckdb")]
    db_path: String,

    /// Comma-separated gamma values to evaluate
    #[arg(long, default_value = "1.0,0.75,0.5,0.25,0.0")]
    gammas: String,

    /// Extra profiles as "name=key1,key2" (repeatable). Replaces defaults if given.
    #[arg(long = "profile")]
    profiles: Vec<String>,

    /// Number of top item nodes used for composition metrics
    #[arg(long, default_value_t = 50)]
    top_k: usize,

    /// Number of named examples printed per domain
    #[arg(long, default_value_t = 5)]
    examples: usize,

    /// Optional JSON output path
    #[arg(long)]
    json: Option<String>,
}

const BUCKETS: [&str; 7] = ["movie", "series", "game", "book", "music", "concept", "creator"];

fn bucket_of(key: &str) -> usize {
    if let Some(d) = item_domain_of(key) {
        return d;
    }
    match key.split(':').next().unwrap_or("") {
        "keyword" | "epoch" => 5,
        _ => 6,
    }
}

fn default_profiles() -> Vec<(String, Vec<String>)> {
    let p = |name: &str, keys: &[&str]| {
        (name.to_string(), keys.iter().map(|s| s.to_string()).collect())
    };
    vec![
        p("solo_game_skyrim", &["game:7811"]),
        p("solo_movie_donnie_darko", &["movie:22697"]),
        p("solo_book_dune", &["book:4"]),
        p("solo_series_dark", &["series:5818"]),
        p("solo_music_echo", &["artist:927"]),
        p("mix_skyrim_booka", &["game:7811", "artist:1968"]),
        p("mix_bladerunner_neuromancer_massive", &["movie:1", "book:2", "artist:12"]),
        p("mix_twinpeaks_radiohead", &["series:5208", "artist:77"]),
    ]
}

#[derive(Serialize, Clone)]
struct Example {
    key: String,
    label: String,
    score: f32,
}

#[derive(Serialize)]
struct ProfileResult {
    gamma: f32,
    profile: String,
    seeds: Vec<String>,
    total_mass: f32,
    /// Share of non-seed mass per bucket (movie, series, game, book, music, concept, creator)
    mass_share: HashMap<String, f32>,
    /// Domain counts among the top-K non-seed item nodes
    topk_counts: HashMap<String, usize>,
    /// Normalized Shannon entropy of the top-K domain composition (0 = single domain, 1 = uniform)
    topk_entropy: f32,
    /// Jaccard overlap of each domain's top-N with the gamma=1.0 baseline (if computed)
    baseline_overlap: HashMap<String, f32>,
    examples: HashMap<String, Vec<Example>>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let gammas: Vec<f32> = args
        .gammas
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let profiles: Vec<(String, Vec<String>)> = if args.profiles.is_empty() {
        default_profiles()
    } else {
        args.profiles
            .iter()
            .filter_map(|p| {
                let (name, keys) = p.split_once('=')?;
                Some((name.to_string(), keys.split(',').map(|s| s.trim().to_string()).collect()))
            })
            .collect()
    };

    let t0 = Instant::now();
    let raw = RawEdges::load_from_duckdb(&args.db_path, None)?;
    eprintln!("[report] raw edges loaded in {:.2}s ({} nodes)", t0.elapsed().as_secs_f32(), raw.num_nodes());

    let mut results: Vec<ProfileResult> = Vec::new();
    // Per (profile, domain) top-N id sets at gamma=1.0, for overlap comparison
    let mut baseline_sets: HashMap<(String, usize), HashSet<String>> = HashMap::new();
    let overlap_n = 20usize;

    for &gamma in &gammas {
        let t = Instant::now();
        let graph = GraphCSR::from_raw(raw.clone(), gamma);
        eprintln!("[report] gamma={gamma:.2} matrix built in {:.2}s", t.elapsed().as_secs_f32());

        for (name, seeds) in &profiles {
            let seed_refs: Vec<&str> = seeds.iter().map(|s| s.as_str()).collect();
            let p = match run_personalized_pagerank_scores(&graph, &seed_refs, 0.85, 25, 1e-6) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[report] skipping {name}: {e}");
                    continue;
                }
            };
            let seed_ids: HashSet<usize> = seeds
                .iter()
                .filter_map(|s| graph.get_node_id(s).map(|i| i as usize))
                .collect();

            let total_mass: f32 = p.iter().sum();
            let mut bucket_mass = [0.0f64; BUCKETS.len()];
            let mut per_domain: Vec<Vec<(usize, f32)>> = vec![Vec::new(); ITEM_DOMAINS.len()];
            let mut items: Vec<(usize, f32)> = Vec::new();

            for (id, &score) in p.iter().enumerate() {
                if seed_ids.contains(&id) || score <= 0.0 {
                    continue;
                }
                let key = &graph.id_to_node[id];
                bucket_mass[bucket_of(key)] += score as f64;
                if let Some(d) = item_domain_of(key) {
                    per_domain[d].push((id, score));
                    items.push((id, score));
                }
            }

            let non_seed: f64 = bucket_mass.iter().sum::<f64>().max(1e-12);
            let mass_share = BUCKETS
                .iter()
                .enumerate()
                .map(|(i, b)| (b.to_string(), (bucket_mass[i] / non_seed) as f32))
                .collect();

            let by_score = |a: &(usize, f32), b: &(usize, f32)| b.1.partial_cmp(&a.1).unwrap();
            items.sort_unstable_by(by_score);
            let mut topk_counts_arr = [0usize; ITEM_DOMAINS.len()];
            for &(id, _) in items.iter().take(args.top_k) {
                if let Some(d) = item_domain_of(&graph.id_to_node[id]) {
                    topk_counts_arr[d] += 1;
                }
            }
            let k_total: usize = topk_counts_arr.iter().sum();
            let entropy = if k_total > 0 {
                let h: f32 = topk_counts_arr
                    .iter()
                    .filter(|&&c| c > 0)
                    .map(|&c| {
                        let q = c as f32 / k_total as f32;
                        -q * q.ln()
                    })
                    .sum();
                h / (ITEM_DOMAINS.len() as f32).ln()
            } else {
                0.0
            };

            let mut examples: HashMap<String, Vec<Example>> = HashMap::new();
            let mut baseline_overlap: HashMap<String, f32> = HashMap::new();
            for (d, list) in per_domain.iter_mut().enumerate() {
                list.sort_unstable_by(by_score);
                let top_set: HashSet<String> = list
                    .iter()
                    .take(overlap_n)
                    .map(|&(id, _)| graph.id_to_node[id].clone())
                    .collect();
                let bkey = (name.clone(), d);
                if (gamma - 1.0).abs() < 1e-6 {
                    baseline_sets.insert(bkey, top_set);
                } else if let Some(base) = baseline_sets.get(&bkey) {
                    let inter = base.intersection(&top_set).count() as f32;
                    let uni = base.union(&top_set).count().max(1) as f32;
                    baseline_overlap.insert(ITEM_DOMAINS[d].to_string(), inter / uni);
                }
                examples.insert(
                    ITEM_DOMAINS[d].to_string(),
                    list.iter()
                        .take(args.examples)
                        .map(|&(id, score)| Example {
                            key: graph.id_to_node[id].clone(),
                            label: String::new(),
                            score,
                        })
                        .collect(),
                );
            }

            results.push(ProfileResult {
                gamma,
                profile: name.clone(),
                seeds: seeds.clone(),
                total_mass,
                mass_share,
                topk_counts: ITEM_DOMAINS
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (d.to_string(), topk_counts_arr[i]))
                    .collect(),
                topk_entropy: entropy,
                baseline_overlap,
                examples,
            });
        }
    }

    // Resolve labels for all example keys in a single query
    let all_keys: HashSet<String> = results
        .iter()
        .flat_map(|r| r.examples.values().flatten().map(|e| e.key.clone()))
        .chain(profiles.iter().flat_map(|(_, s)| s.clone()))
        .collect();
    let labels = resolve_labels(&args.db_path, &all_keys).unwrap_or_default();
    for r in results.iter_mut() {
        for list in r.examples.values_mut() {
            for e in list.iter_mut() {
                e.label = labels.get(&e.key).cloned().unwrap_or_default();
            }
        }
    }

    print_report(&results, &profiles, &labels, &gammas, args.top_k);

    if let Some(path) = &args.json {
        std::fs::write(path, serde_json::to_string_pretty(&results)?)?;
        eprintln!("[report] json written to {path}");
    }
    Ok(())
}

fn resolve_labels(db_path: &str, keys: &HashSet<String>) -> Result<HashMap<String, String>> {
    let conn = Connection::open_with_flags(db_path, Config::default().access_mode(AccessMode::ReadOnly)?)?;
    let list = keys
        .iter()
        .map(|k| format!("'{}'", k.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    if list.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT node_id, label FROM v_graph_nodes WHERE node_id IN ({list})"
    ))?;
    let mut rows = stmt.query([])?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next()? {
        let k: String = row.get(0)?;
        let l: Option<String> = row.get(1)?;
        out.insert(k, l.unwrap_or_default());
    }
    Ok(out)
}

fn print_report(
    results: &[ProfileResult],
    profiles: &[(String, Vec<String>)],
    labels: &HashMap<String, String>,
    gammas: &[f32],
    top_k: usize,
) {
    for (name, seeds) in profiles {
        let seed_names: Vec<String> = seeds
            .iter()
            .map(|s| format!("{} ({})", labels.get(s).cloned().unwrap_or_default(), s))
            .collect();
        println!("\n=== {name} :: {}", seed_names.join(" + "));
        println!(
            "{:>5} | {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7} | top{top_k}: {:>3} {:>3} {:>3} {:>3} {:>3}  H    | overlap@20 vs γ=1 (mov ser gam boo mus)",
            "γ", "movie", "series", "game", "book", "music", "concept", "creator", "mov", "ser", "gam", "boo", "mus"
        );
        for &g in gammas {
            let Some(r) = results.iter().find(|r| r.profile == *name && (r.gamma - g).abs() < 1e-6) else {
                continue;
            };
            let ms = |b: &str| r.mass_share.get(b).copied().unwrap_or(0.0) * 100.0;
            let tc = |d: &str| r.topk_counts.get(d).copied().unwrap_or(0);
            let ov = |d: &str| {
                r.baseline_overlap
                    .get(d)
                    .map(|v| format!("{:.2}", v))
                    .unwrap_or_else(|| " -  ".to_string())
            };
            println!(
                "{:>5.2} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}% {:>6.1}% {:>6.1}% |        {:>3} {:>3} {:>3} {:>3} {:>3}  {:.2} | {} {} {} {} {}",
                g,
                ms("movie"), ms("series"), ms("game"), ms("book"), ms("music"), ms("concept"), ms("creator"),
                tc("movie"), tc("series"), tc("game"), tc("book"), tc("music"),
                r.topk_entropy,
                ov("movie"), ov("series"), ov("game"), ov("book"), ov("music"),
            );
        }
        for &g in gammas {
            let Some(r) = results.iter().find(|r| r.profile == *name && (r.gamma - g).abs() < 1e-6) else {
                continue;
            };
            println!("  -- γ={g:.2} examples");
            for d in ITEM_DOMAINS {
                let ex = r.examples.get(d).map(|v| {
                    v.iter()
                        .map(|e| if e.label.is_empty() { e.key.clone() } else { e.label.clone() })
                        .collect::<Vec<_>>()
                        .join(" | ")
                });
                println!("     {:<7} {}", d, ex.unwrap_or_default());
            }
        }
    }
}
