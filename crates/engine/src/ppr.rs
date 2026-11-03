use crate::graph::GraphCSR;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoredNode {
    pub node_key: String,
    pub score: f32,
}

/// Executa o Personalized PageRank (Random Walk with Restart) sobre a matriz esparsa CSC
/// e retorna o vetor bruto de probabilidades estacionárias `p` (tamanho `graph.num_nodes`).
pub fn run_personalized_pagerank_scores(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<f32>> {
    let n = graph.num_nodes;
    let mut seed_indices: Vec<u32> = Vec::new();

    for &key in seed_keys {
        if let Some(id) = graph.get_node_id(key) {
            seed_indices.push(id);
        }
    }

    if seed_indices.is_empty() {
        anyhow::bail!("Nenhuma das sementes especificadas foi encontrada no grafo carregado.");
    }

    let mut s = vec![0.0f32; n];
    let seed_prob = 1.0f32 / seed_indices.len() as f32;
    for &id in &seed_indices {
        s[id as usize] = seed_prob;
    }

    let mut p = s.clone();
    let mut p_next = vec![0.0f32; n];
    let restart_factor = 1.0f32 - alpha;

    for _iter in 0..max_iter {
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

    Ok(p)
}

/// Extrai os top-K nós de `p` que começam com `prefix` (ou qualquer prefixo se `prefix.is_empty()`),
/// ignorando nós presentes em `exclude_ids`, em tempo O(N) via `select_nth_unstable_by`.
pub fn top_k_from_scores(
    graph: &GraphCSR,
    p: &[f32],
    exclude_ids: &HashSet<u32>,
    prefix: &str,
    limit: Option<usize>,
) -> Vec<ScoredNode> {
    let mut candidates: Vec<(usize, f32)> = p
        .iter()
        .copied()
        .enumerate()
        .filter(|&(id, score)| {
            if score <= 1e-7 {
                return false;
            }
            if exclude_ids.contains(&(id as u32)) {
                return false;
            }
            if !prefix.is_empty() && !graph.id_to_node[id].starts_with(prefix) {
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

    candidates
        .into_iter()
        .map(|(id, score)| ScoredNode {
            node_key: graph.id_to_node[id].clone(),
            score,
        })
        .collect()
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

/// Executa o Personalized PageRank com filtragem in-place por prefixo e seleção top-K parcial (O(N)).
pub fn run_personalized_pagerank_filtered(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
    target_prefix: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ScoredNode>> {
    let p = run_personalized_pagerank_scores(graph, seed_keys, alpha, max_iter, tol)?;

    let mut seed_id_set: HashSet<u32> = HashSet::new();
    if limit.is_some() {
        for &key in seed_keys {
            if let Some(id) = graph.get_node_id(key) {
                seed_id_set.insert(id);
            }
        }
    }

    Ok(top_k_from_scores(
        graph,
        &p,
        &seed_id_set,
        target_prefix.unwrap_or(""),
        limit,
    ))
}
