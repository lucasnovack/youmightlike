use crate::graph::GraphCSR;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoredNode {
    pub node_key: String,
    pub score: f32,
}

/// Executa o Personalized PageRank (Random Walk with Restart) sobre a matriz esparsa CSC.
/// Cada iteração roda 100% paralela sobre todos os núcleos da CPU via Rayon.
pub fn run_personalized_pagerank(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<ScoredNode>> {
    run_personalized_pagerank_filtered(graph, seed_keys, alpha, max_iter, tol, None, None)
}

/// Executa o Personalized PageRank com filtragem in-place por prefixo e seleção top-K parcial (O(N)),
/// evitando alocações desnecessárias de `String` para centenas de milhares de nós descartados.
pub fn run_personalized_pagerank_filtered(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
    target_prefix: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ScoredNode>> {
    let n = graph.num_nodes;
    let mut seed_indices: Vec<u32> = Vec::new();
    let mut seed_id_set: HashSet<u32> = HashSet::new();

    for &key in seed_keys {
        if let Some(id) = graph.get_node_id(key) {
            seed_indices.push(id);
            seed_id_set.insert(id);
        }
    }

    if seed_indices.is_empty() {
        anyhow::bail!("Nenhuma das sementes especificadas foi encontrada no grafo carregado.");
    }

    // Inicializa o vetor semente 's' com distribuição uniforme entre as sementes
    let mut s = vec![0.0f32; n];
    let seed_prob = 1.0f32 / seed_indices.len() as f32;
    for &id in &seed_indices {
        s[id as usize] = seed_prob;
    }

    let mut p = s.clone();
    let mut p_next = vec![0.0f32; n];
    let restart_factor = 1.0f32 - alpha;

    for _iter in 0..max_iter {
        // Multiplicação P^T * p paralelizada por chunks de nós de destino v
        p_next.par_iter_mut().enumerate().for_each(|(v, next_val)| {
            let start = graph.indptr[v];
            let end = graph.indptr[v + 1];

            let mut incoming_prob = 0.0f32;
            for i in start..end {
                let u = graph.indices[i] as usize;
                let prob = graph.weights[i];
                incoming_prob += p[u] * prob;
            }

            *next_val = alpha * incoming_prob + restart_factor * s[v];
        });

        // Critério de parada: norma L1 do erro ||p^{(k+1)} - p^{(k)}||_1 < tol
        let err: f32 = p
            .par_iter()
            .zip(p_next.par_iter())
            .map(|(&old, &new)| (old - new).abs())
            .sum();

        std::mem::swap(&mut p, &mut p_next);

        if err < tol {
            break;
        }
    }

    let pfx = target_prefix.unwrap_or("");
    let filter_seeds = limit.is_some();

    let mut candidates: Vec<(usize, f32)> = p
        .into_iter()
        .enumerate()
        .filter(|&(id, score)| {
            if score <= 1e-7 {
                return false;
            }
            if filter_seeds && seed_id_set.contains(&(id as u32)) {
                return false;
            }
            if !pfx.is_empty() && !graph.id_to_node[id].starts_with(pfx) {
                return false;
            }
            true
        })
        .collect();

    let cmp = |a: &(usize, f32), b: &(usize, f32)| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    };

    if let Some(k) = limit {
        if candidates.len() > k {
            candidates.select_nth_unstable_by(k, cmp);
            candidates.truncate(k);
        }
    }

    candidates.sort_unstable_by(cmp);

    let scored: Vec<ScoredNode> = candidates
        .into_iter()
        .map(|(id, score)| ScoredNode {
            node_key: graph.id_to_node[id].clone(),
            score,
        })
        .collect();

    Ok(scored)
}
