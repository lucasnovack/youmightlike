use anyhow::{Context, Result};
use duckdb::{AccessMode, Config, Connection};
use std::collections::{HashMap, HashSet};

/// Original directed semantic edge in the graph (for explanations and in-memory subgraph assembly).
#[derive(Debug, Clone, Copy)]
pub struct DirectedEdge<'a> {
    pub target_id: u32,
    pub target_key: &'a str,
    pub relation_type: &'a str,
    pub base_weight: f32,
}

/// In-memory optimized sparse graph in CSC (Compressed Sparse Column) format
/// for parallel Personalized PageRank computation via Rayon, accompanied by
/// a CSR index of original directed semantic edges for real-time subgraphs and path explanations.
pub struct GraphCSR {
    pub num_nodes: usize,
    pub num_edges: usize,
    /// Pointer offsets for each node in the CSC diffusion index vector (length: num_nodes + 1)
    pub indptr: Vec<usize>,
    /// Source node indices for each incoming edge (length: num_edges)
    pub indices: Vec<u32>,
    /// Normalized stochastic Markov transition probabilities: sum(P[u, :]) == 1.0
    pub weights: Vec<f32>,
    /// Map from string key (e.g. 'artist:11', 'track:38862') to internal integer ID
    pub node_to_id: HashMap<String, u32>,
    /// Inverse map from internal integer ID to string key
    pub id_to_node: Vec<String>,

    /// Interned relation type names (e.g. 'has_tag', 'similar_to', 'has_director')
    pub relation_types: Vec<String>,
    /// CSR pointer offsets for original outgoing directed edges per source node (length: num_nodes + 1)
    pub out_indptr: Vec<usize>,
    /// Destination node ID for each original directed edge
    pub out_targets: Vec<u32>,
    /// Relation type ID in `relation_types`
    pub out_rel_ids: Vec<u8>,
    /// Original base weight of the directed edge
    pub out_weights: Vec<f32>,
}

impl GraphCSR {
    /// Loads the full semantic graph (`graph_semantic_edges`) including `relation_type`.
    pub fn load_semantic_graph(db_path: &str) -> Result<Self> {
        Self::load_from_duckdb(
            db_path,
            Some("SELECT source_id, target_id, base_weight, relation_type FROM graph_semantic_edges WHERE base_weight > 0.0"),
        )
    }

    /// Loads graph edges from DuckDB and builds the in-memory CSC matrix with Adamic-Adar / IDF attenuation,
    /// along with the directed CSR index for instant topological inspection.
    pub fn load_from_duckdb(
        db_path: &str,
        query_edges_sql: Option<&str>,
    ) -> Result<Self> {
        let conn = Connection::open_with_flags(
            db_path,
            Config::default().access_mode(AccessMode::ReadOnly)?,
        )
        .context("Failed to open DuckDB database in read_only mode")?;

        conn.execute_batch("PRAGMA threads=8; PRAGMA memory_limit='16GB';")?;

        let sql = query_edges_sql.unwrap_or(
            "SELECT source_id, target_id, base_weight, relation_type FROM graph_semantic_edges WHERE base_weight > 0.0",
        );

        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query([])?;

        let mut node_to_id: HashMap<String, u32> = HashMap::new();
        let mut id_to_node: Vec<String> = Vec::new();

        let mut rel_to_id: HashMap<String, u8> = HashMap::new();
        let mut relation_types: Vec<String> = Vec::new();

        let get_or_create_id =
            |key: &str, node_map: &mut HashMap<String, u32>, id_list: &mut Vec<String>| -> u32 {
                if let Some(&id) = node_map.get(key) {
                    id
                } else {
                    let id = id_list.len() as u32;
                    node_map.insert(key.to_string(), id);
                    id_list.push(key.to_string());
                    id
                }
            };

        let get_or_create_rel =
            |rel: &str, rel_map: &mut HashMap<String, u8>, rel_list: &mut Vec<String>| -> u8 {
                if let Some(&id) = rel_map.get(rel) {
                    id
                } else {
                    let id = rel_list.len() as u8;
                    rel_map.insert(rel.to_string(), id);
                    rel_list.push(rel.to_string());
                    id
                }
            };

        // Raw edge tuples for symmetric diffusion (src, dst, weight)
        let mut raw_edges: Vec<(u32, u32, f32)> = Vec::new();
        // Original directed tuples (src, dst, rel_id, weight)
        let mut directed_edges: Vec<(u32, u32, u8, f32)> = Vec::new();
        let mut seen_directed: HashSet<(u32, u32, u8)> = HashSet::new();
        let mut in_degrees: Vec<u32> = Vec::new();

        let push_edge = |src_key: &str,
                             dst_key: &str,
                             rel_str: &str,
                             w: f32,
                             node_to_id: &mut HashMap<String, u32>,
                             id_to_node: &mut Vec<String>,
                             rel_to_id: &mut HashMap<String, u8>,
                             relation_types: &mut Vec<String>,
                             in_degrees: &mut Vec<u32>,
                             raw_edges: &mut Vec<(u32, u32, f32)>,
                             directed_edges: &mut Vec<(u32, u32, u8, f32)>,
                             seen_directed: &mut HashSet<(u32, u32, u8)>| {
            let u = get_or_create_id(src_key, node_to_id, id_to_node);
            let v = get_or_create_id(dst_key, node_to_id, id_to_node);
            let rel_id = get_or_create_rel(rel_str, rel_to_id, relation_types);

            let max_idx = u.max(v) as usize;
            if in_degrees.len() <= max_idx {
                in_degrees.resize(max_idx + 1, 0);
            }

            if !seen_directed.insert((u, v, rel_id)) {
                return;
            }

            directed_edges.push((u, v, rel_id, w));

            in_degrees[v as usize] += 1;
            raw_edges.push((u, v, w));

            if u != v {
                in_degrees[u as usize] += 1;
                raw_edges.push((v, u, w));
            }
        };

        while let Some(row) = rows.next()? {
            let src_key: String = row.get(0)?;
            let dst_key: String = row.get(1)?;
            let base_w: f64 = row.get(2)?;
            let rel_str: String = row.get(3).unwrap_or_else(|_| "connected".to_string());

            push_edge(
                &src_key,
                &dst_key,
                &rel_str,
                base_w as f32,
                &mut node_to_id,
                &mut id_to_node,
                &mut rel_to_id,
                &mut relation_types,
                &mut in_degrees,
                &mut raw_edges,
                &mut directed_edges,
                &mut seen_directed,
            );
        }

        let num_nodes = id_to_node.len();
        let num_edges = raw_edges.len();

        if num_nodes == 0 {
            anyhow::bail!("No edges found in DuckDB query.");
        }

        // 1. Applies in-memory Adamic-Adar / IDF attenuation: w = base_w / ln(1 + degree(v))
        // and calculates out-degree sums for each node u for stochastic normalization
        let mut out_sums: Vec<f32> = vec![0.0f32; num_nodes];
        let mut adjusted_edges: Vec<(u32, u32, f32)> = Vec::with_capacity(num_edges);

        for (u, v, base_w) in raw_edges {
            let deg = in_degrees[v as usize].max(1) as f32;
            let idf = 1.0f32 / (1.0f32 + deg).ln().max(1.0f32);
            let eff_w = base_w * idf;
            out_sums[u as usize] += eff_w;
            adjusted_edges.push((u, v, eff_w));
        }

        // 2. Normalizes to stochastic Markov transition probabilities: P[u, v]
        // and groups by destination node v (CSC format - in-edges)
        let mut in_adjacency: Vec<Vec<(u32, f32)>> = vec![Vec::new(); num_nodes];
        for (u, v, eff_w) in adjusted_edges {
            let sum_u = out_sums[u as usize];
            let prob = if sum_u > 0.0 { eff_w / sum_u } else { 0.0 };
            in_adjacency[v as usize].push((u, prob));
        }

        // 3. Packs into contiguous, cache-friendly CSC structures for PPR
        let mut indptr: Vec<usize> = Vec::with_capacity(num_nodes + 1);
        let mut indices: Vec<u32> = Vec::with_capacity(num_edges);
        let mut weights: Vec<f32> = Vec::with_capacity(num_edges);

        let mut offset = 0;
        indptr.push(offset);

        for in_edges in in_adjacency {
            offset += in_edges.len();
            indptr.push(offset);
            for (src_u, p_val) in in_edges {
                indices.push(src_u);
                weights.push(p_val);
            }
        }

        // 4. Builds CSR index of original directed semantic edges by source node
        let mut out_adj: Vec<Vec<(u32, u8, f32)>> = vec![Vec::new(); num_nodes];
        for (u, v, rel_id, w) in directed_edges {
            out_adj[u as usize].push((v, rel_id, w));
        }

        let num_directed = out_adj.iter().map(|v| v.len()).sum();
        let mut out_indptr: Vec<usize> = Vec::with_capacity(num_nodes + 1);
        let mut out_targets: Vec<u32> = Vec::with_capacity(num_directed);
        let mut out_rel_ids: Vec<u8> = Vec::with_capacity(num_directed);
        let mut out_weights: Vec<f32> = Vec::with_capacity(num_directed);

        let mut out_offset = 0;
        out_indptr.push(out_offset);
        for edges in out_adj {
            out_offset += edges.len();
            out_indptr.push(out_offset);
            for (dst_v, rel_id, w) in edges {
                out_targets.push(dst_v);
                out_rel_ids.push(rel_id);
                out_weights.push(w);
            }
        }

        Ok(Self {
            num_nodes,
            num_edges,
            indptr,
            indices,
            weights,
            node_to_id,
            id_to_node,
            relation_types,
            out_indptr,
            out_targets,
            out_rel_ids,
            out_weights,
        })
    }

    /// Returns the integer ID corresponding to a node key (e.g. 'artist:11')
    pub fn get_node_id(&self, key: &str) -> Option<u32> {
        self.node_to_id.get(key).copied()
    }

    /// Returns the string key of a node given its integer ID
    pub fn get_node_key(&self, id: u32) -> Option<&str> {
        self.id_to_node.get(id as usize).map(|s| s.as_str())
    }

    /// Iterates over original directed edges outgoing from node `u` (integer ID)
    pub fn out_edges(&self, u: u32) -> impl Iterator<Item = DirectedEdge<'_>> {
        let u_idx = u as usize;
        let (start, end) = if u_idx < self.num_nodes {
            (self.out_indptr[u_idx], self.out_indptr[u_idx + 1])
        } else {
            (0, 0)
        };
        (start..end).map(move |i| {
            let target_id = self.out_targets[i];
            let rel_id = self.out_rel_ids[i] as usize;
            DirectedEdge {
                target_id,
                target_key: &self.id_to_node[target_id as usize],
                relation_type: &self.relation_types[rel_id],
                base_weight: self.out_weights[i],
            }
        })
    }

    /// Iterates over original directed edges outgoing from `node_key` (e.g. 'artist:11')
    pub fn out_edges_by_key(&self, node_key: &str) -> Vec<DirectedEdge<'_>> {
        if let Some(u) = self.get_node_id(node_key) {
            self.out_edges(u).collect()
        } else {
            Vec::new()
        }
    }
}
