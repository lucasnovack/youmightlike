use crate::graph::GraphCSR;
use anyhow::Result;
use rayon::prelude::*;

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

    // Coleta e ordena os nós pelo score final de afinidade
    let mut scored: Vec<ScoredNode> = p
        .into_iter()
        .enumerate()
        .filter(|&(_, score)| score > 1e-7)
        .map(|(id, score)| ScoredNode {
            node_key: graph.id_to_node[id].clone(),
            score,
        })
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    Ok(scored)
}
