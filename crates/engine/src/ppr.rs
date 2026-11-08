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

/// Executes Personalized PageRank with Pixie-style Multi-Hit Booster (Eksombatchai et al., WWW 2018).
/// When multiple distinct seeds Q are provided, individual stationary diffusion vectors p_q are computed
/// for each seed independently, and then non-linearly combined:
///
///   p_multi[v] = ( \sum_{q \in Q} \sqrt{w_q \cdot p_q[v]} )^2
///
/// Candidates that receive energy from multiple disparate query seeds receive a quadratic boost,
/// naturally surfacing cross-domain cultural bridges while reducing to standard PPR when |Q| == 1.
pub fn run_personalized_pagerank_multi_hit(
    graph: &GraphCSR,
    seed_weights: &[(&str, f32)],
    alpha: f32,
    max_iter: usize,
    tol: f32,
) -> Result<Vec<f32>> {
    let mut resolved: Vec<(&str, f32)> = Vec::new();
    let mut total_w = 0.0f32;

    for &(key, w) in seed_weights {
        if w > 0.0 && graph.get_node_id(key).is_some() {
            resolved.push((key, w));
            total_w += w;
        }
    }

    if resolved.is_empty() || total_w <= 0.0 {
        anyhow::bail!("None of the specified seed keys were found in the loaded graph.");
    }

    // If there is only 1 seed, multi-hit reduces to linear PPR
    if resolved.len() == 1 {
        return run_personalized_pagerank_scores_weighted(graph, &resolved, alpha, max_iter, tol);
    }

    // Normalize weights so sum(w_q) = 1.0
    let normalized_seeds: Vec<(&str, f32)> = resolved
        .iter()
        .map(|&(k, w)| (k, w / total_w))
        .collect();

    // Group into at most 8 representative multi-seed walkers to keep diffusion
    // sub-second even with 30+ seeds, while preventing correlation explosion
    const MAX_WALKERS: usize = 8;
    let seed_groups = cluster_seeds_for_multi_hit(&normalized_seeds, MAX_WALKERS);

    let group_weights: Vec<f32> = seed_groups
        .iter()
        .map(|grp| grp.iter().map(|s| s.1).sum::<f32>())
        .collect();

    // Compute PPR vectors for each cluster group in parallel
    let p_vectors: Vec<Vec<f32>> = seed_groups
        .par_iter()
        .map(|grp| {
            run_personalized_pagerank_scores_weighted(graph, grp, alpha, max_iter, tol)
        })
        .collect::<Result<Vec<_>>>()?;

    let n = graph.num_nodes;
    let mut p_boosted = vec![0.0f32; n];
    let noise_floor = (0.5f32 / (n as f32)).min(1e-5);

    // Compute ( \sum_g \sqrt{W_g * (p_g[v] - noise)_+} )^2 in parallel across all nodes,
    // applying degree dampening to eliminate hub / supernode monopolies
    p_boosted
        .par_iter_mut()
        .enumerate()
        .for_each(|(v, out)| {
            let mut sum_sqrt = 0.0f32;
            for (i, p_vec) in p_vectors.iter().enumerate() {
                let w = group_weights[i];
                let val = (p_vec[v] - noise_floor).max(0.0);
                if val > 0.0 {
                    sum_sqrt += (w * val).sqrt();
                }
            }
            let raw_boosted = sum_sqrt * sum_sqrt;
            if raw_boosted > 0.0 {
                let deg = (graph.indptr[v + 1] - graph.indptr[v]).max(1) as f32;
                // Popularity/Hub dampening: penalizes ubiquitous hubs (blockbusters with hundreds of edges)
                // so specific cultural matches shine through without generic hubs hogging all top spots.
                let dampening = if deg > 40.0 {
                    1.0 / (1.0 + (deg / 40.0).ln())
                } else {
                    1.0
                };
                *out = raw_boosted * dampening;
            } else {
                *out = 0.0;
            }
        });

    // Re-normalize to conserve stochastic probability mass (\sum p_i = 1.0)
    let total_boosted: f32 = p_boosted.par_iter().sum();
    if total_boosted > 0.0 {
        p_boosted.par_iter_mut().for_each(|val| {
            *val /= total_boosted;
        });
    }

    Ok(p_boosted)
}

/// Partitions seeds into at most `max_groups` balanced cultural clusters
/// based on domain prefix to keep multi-hit diffusion fast and prevent
/// correlation amplification when many seeds are provided.
fn cluster_seeds_for_multi_hit<'a>(
    seeds: &[(&'a str, f32)],
    max_groups: usize,
) -> Vec<Vec<(&'a str, f32)>> {
    if seeds.len() <= max_groups {
        return seeds.iter().map(|&s| vec![s]).collect();
    }

    use std::collections::HashMap;
    let mut by_domain: HashMap<&str, Vec<(&str, f32)>> = HashMap::new();
    for &s in seeds {
        let prefix = s.0.split(':').next().unwrap_or("other");
        let domain_key = match prefix {
            "artist" | "track" | "album" => "music",
            other => other,
        };
        by_domain.entry(domain_key).or_default().push(s);
    }

    let mut dom_entries: Vec<(&str, Vec<(&str, f32)>)> = by_domain.into_iter().collect();
    dom_entries.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut groups: Vec<Vec<(&str, f32)>> = Vec::new();
    let num_domains = dom_entries.len();

    if num_domains >= max_groups {
        for (i, entry) in dom_entries.into_iter().enumerate() {
            if i < max_groups - 1 {
                groups.push(entry.1);
            } else if groups.len() < max_groups {
                groups.push(entry.1);
            } else {
                groups.last_mut().unwrap().extend(entry.1);
            }
        }
    } else {
        let mut remaining_slots = max_groups;
        let mut remaining_domains = num_domains;

        for (_dom, dom_seeds) in dom_entries {
            let slots_for_dom = (remaining_slots / remaining_domains).max(1).min(dom_seeds.len());
            remaining_slots = remaining_slots.saturating_sub(slots_for_dom);
            remaining_domains -= 1;

            if slots_for_dom == 1 {
                groups.push(dom_seeds);
            } else {
                let chunk_size = (dom_seeds.len() + slots_for_dom - 1) / slots_for_dom;
                for chunk in dom_seeds.chunks(chunk_size) {
                    groups.push(chunk.to_vec());
                }
            }
        }
    }

    groups
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
    use crate::graph::RawEdges;

    fn sample_graph() -> GraphCSR {
        let mut raw = RawEdges::default();
        raw.push("movie:1", "keyword:cyberpunk", "has_keyword", 1.0);
        raw.push("movie:1", "person:1", "has_director", 1.0);
        raw.push("game:1", "keyword:cyberpunk", "has_tag", 1.0);
        raw.push("game:1", "developer:1", "developed_by", 1.0);
        raw.push("artist:1", "keyword:cyberpunk", "has_tag", 1.0);
        raw.push("book:1", "keyword:cyberpunk", "has_tag", 1.0);
        raw.push("series:1", "person:1", "has_director", 1.0);
        GraphCSR::from_raw(raw, 0.5)
    }

    #[test]
    fn test_ppr_probability_conservation_single_seed() {
        let graph = sample_graph();
        for alpha in [0.0, 0.15, 0.50, 0.85, 0.95] {
            let p = run_personalized_pagerank_scores(&graph, &["movie:1"], alpha, 150, 1e-6)
                .expect("PPR execution should succeed");

            let sum: f32 = p.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "Probability sum must be 1.0 for alpha={alpha}, got {sum}"
            );
            assert!(
                p.iter().all(|&x| x >= 0.0),
                "All probabilities must be non-negative for alpha={alpha}"
            );
        }
    }

    #[test]
    fn test_ppr_probability_conservation_weighted_seeds() {
        let graph = sample_graph();
        let seed_weights = [("movie:1", 0.65), ("game:1", 0.35)];

        for alpha in [0.1, 0.5, 0.85] {
            let p =
                run_personalized_pagerank_scores_weighted(&graph, &seed_weights, alpha, 150, 1e-6)
                    .expect("Weighted PPR execution should succeed");

            let sum: f32 = p.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "Probability sum must be 1.0 for weighted seeds with alpha={alpha}, got {sum}"
            );
        }
    }

    #[test]
    fn test_ppr_teleport_dominance_when_alpha_zero() {
        let graph = sample_graph();
        let p = run_personalized_pagerank_scores(&graph, &["movie:1"], 0.0, 10, 1e-6)
            .expect("PPR with alpha=0 should succeed");

        let movie_id = graph.get_node_id("movie:1").expect("movie:1 must exist") as usize;
        assert!(
            (p[movie_id] - 1.0).abs() < 1e-5,
            "When alpha=0, 100% of probability must stay on the seed"
        );

        for (i, &score) in p.iter().enumerate() {
            if i != movie_id {
                assert_eq!(score, 0.0, "Non-seed node {i} must have 0.0 probability");
            }
        }
    }

    #[test]
    fn test_top_k_from_scores_sorting_and_filtering() {
        let graph = sample_graph();
        let p = run_personalized_pagerank_scores(&graph, &["movie:1"], 0.85, 100, 1e-6)
            .expect("PPR should succeed");

        let mut exclude = HashSet::new();
        exclude.insert(graph.get_node_id("movie:1").unwrap());

        // Extract top-2 without prefix filter
        let top2 = top_k_from_scores(&graph, &p, &exclude, "", Some(2));
        assert_eq!(top2.len(), 2);
        assert!(top2[0].score >= top2[1].score, "Must be descending order");
        assert!(
            !top2.iter().any(|n| n.node_key == "movie:1"),
            "Excluded seed must not be in top-k"
        );

        // Extract with prefix filter "game:"
        let games = top_k_from_scores(&graph, &p, &exclude, "game:", Some(5));
        assert!(!games.is_empty(), "Must find at least one game");
        for g in &games {
            assert!(
                g.node_key.starts_with("game:"),
                "Result {} must match prefix 'game:'",
                g.node_key
            );
        }
    }

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

    #[test]
    fn test_multi_hit_booster_probability_conservation_and_intersection_boost() {
        let graph = sample_graph();
        let seed_weights = [("movie:1", 0.5), ("game:1", 0.5)];

        // Compute standard linear weighted PPR
        let p_linear = run_personalized_pagerank_scores_weighted(
            &graph,
            &seed_weights,
            0.85,
            150,
            1e-6,
        )
        .expect("linear PPR should succeed");

        // Compute multi-hit boosted PPR
        let p_multi = run_personalized_pagerank_multi_hit(
            &graph,
            &seed_weights,
            0.85,
            150,
            1e-6,
        )
        .expect("multi-hit PPR should succeed");

        // 1. Verify stochastic conservation: sum(p_multi) = 1.0
        let sum_multi: f32 = p_multi.iter().sum();
        assert!(
            (sum_multi - 1.0).abs() < 1e-4,
            "Multi-hit probability sum must be 1.0, got {sum_multi}"
        );

        // 2. keyword:cyberpunk is connected to BOTH movie:1 and game:1
        // person:1 is connected ONLY to movie:1
        // developer:1 is connected ONLY to game:1
        let bridge_id = graph.get_node_id("keyword:cyberpunk").unwrap() as usize;
        let single_movie_branch = graph.get_node_id("person:1").unwrap() as usize;

        // Ratio of bridge energy to single branch in multi-hit should exceed linear ratio!
        let ratio_linear = p_linear[bridge_id] / p_linear[single_movie_branch];
        let ratio_multi = p_multi[bridge_id] / p_multi[single_movie_branch];

        assert!(
            ratio_multi > ratio_linear,
            "Multi-hit booster must amplify intersection node ratio ({ratio_multi} > {ratio_linear})"
        );
    }
}

