use crate::graph::GraphCSR;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoredNode {
    pub node_key: String,
    pub score: f32,
}

/// Executes Personalized PageRank (Random Walk with Restart) over the sparse CSC matrix
/// and returns the raw stationary probability vector `p` (length `graph.num_nodes`).
pub fn run_personalized_pagerank_scores(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<f32>> {
    let seed_weights: Vec<(&str, f32)> = seed_keys.iter().map(|&k| (k, 1.0f32)).collect();
    run_personalized_pagerank_scores_weighted(graph, &seed_weights, alpha, max_iter, tol)
}

/// Executes Personalized PageRank with explicit seed probability weights `(node_key, weight)`.
/// Weights are normalized so that sum(weights) = 1.0.
pub fn run_personalized_pagerank_scores_weighted(
    graph: &GraphCSR,
    seed_weights: &[(&str, f32)],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<f32>> {
    let n = graph.num_nodes;
    let mut resolved: Vec<(u32, f32)> = Vec::new();
    let mut total_w = 0.0f32;

    for &(key, w) in seed_weights {
        if w > 0.0 {
            if let Some(id) = graph.get_node_id(key) {
                resolved.push((id, w));
                total_w += w;
            }
        }
    }

    if resolved.is_empty() || total_w <= 0.0 {
        anyhow::bail!("None of the specified seed keys were found in the loaded graph.");
    }

    let mut s = vec![0.0f32; n];
    for (id, w) in resolved {
        s[id as usize] += w / total_w;
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

/// Applies repulsive negative feedback penalty:
/// p_pos[v] = max(0.0, p_pos[v] - beta * p_neg[v])
/// Executed in parallel using Rayon across all nodes.
pub fn penalize_negative_scores(p_pos: &mut [f32], p_neg: &[f32], beta: f32) {
    p_pos
        .par_iter_mut()
        .zip(p_neg.par_iter())
        .for_each(|(pos, &neg)| {
            *pos = (*pos - beta * neg).max(0.0);
        });
}

/// Extracts top-K nodes from `p` that start with `prefix` (or any prefix if `prefix.is_empty()`),
/// ignoring nodes present in `exclude_ids`, in O(N) time via `select_nth_unstable_by`.
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

/// Executes Personalized PageRank (Random Walk with Restart) over the sparse CSC matrix.
/// Each iteration runs 100% in parallel across all CPU cores via Rayon.
pub fn run_personalized_pagerank(
    graph: &GraphCSR,
    seed_keys: &[&str],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<ScoredNode>> {
    run_personalized_pagerank_filtered(graph, seed_keys, alpha, max_iter, tol, None, None)
}

/// Executes Personalized PageRank with in-place prefix filtering and partial top-K selection (O(N)).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_penalize_negative_scores() {
        let mut p_pos = vec![0.5f32, 0.3, 0.2, 0.05];
        let p_neg = vec![0.1f32, 0.4, 0.0, 0.1];
        let beta = 1.5f32;

        penalize_negative_scores(&mut p_pos, &p_neg, beta);

        // index 0: 0.5 - 1.5 * 0.1 = 0.35
        assert!((p_pos[0] - 0.35).abs() < 1e-6);
        // index 1: 0.3 - 1.5 * 0.4 = -0.3 -> clamped to 0.0
        assert_eq!(p_pos[1], 0.0);
        // index 2: 0.2 - 0 = 0.2
        assert!((p_pos[2] - 0.2).abs() < 1e-6);
        // index 3: 0.05 - 1.5 * 0.1 = -0.1 -> clamped to 0.0
        assert_eq!(p_pos[3], 0.0);
    }
}

