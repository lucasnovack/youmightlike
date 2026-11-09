use anyhow::{Context, Result};
use duckdb::{AccessMode, Config, Connection};
use std::collections::{HashMap, HashSet};

/// Environment variable that controls the inter-domain balancing exponent (gamma).
pub const DOMAIN_GAMMA_ENV: &str = "YML_DOMAIN_GAMMA";

/// Default inter-domain balancing exponent.
/// `1.0` reproduces the legacy behavior (no rebalancing); `0.0` splits mass equally across item domains.
pub const DEFAULT_DOMAIN_GAMMA: f32 = 1.0;

/// Item (recommendable work) domains used by inter-domain normalization.
pub const ITEM_DOMAINS: [&str; 5] = ["movie", "series", "game", "book", "music"];
const NUM_ITEM_DOMAINS: usize = ITEM_DOMAINS.len();
const NO_DOMAIN: u8 = u8::MAX;

/// Maps a node key (e.g. `artist:11`) to its item domain index in [`ITEM_DOMAINS`],
/// or `None` for non-item nodes (keywords, epochs, people, developers, authors).
pub fn item_domain_of(key: &str) -> Option<usize> {
    let prefix = key.split(':').next().unwrap_or("");
    match prefix {
        "movie" => Some(0),
        "series" => Some(1),
        "game" => Some(2),
        "book" => Some(3),
        "artist" | "track" | "album" => Some(4),
        _ => None,
    }
}

/// Reads the balancing exponent from `YML_DOMAIN_GAMMA`, falling back to [`DEFAULT_DOMAIN_GAMMA`].
pub fn domain_gamma_from_env() -> f32 {
    std::env::var(DOMAIN_GAMMA_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|g| g.clamp(0.0, 1.0))
        .unwrap_or(DEFAULT_DOMAIN_GAMMA)
}

/// Original directed semantic edge in the graph (for explanations and in-memory subgraph assembly).
#[derive(Debug, Clone, Copy)]
pub struct DirectedEdge<'a> {
    pub target_id: u32,
    pub target_key: &'a str,
    pub relation_type: &'a str,
    pub base_weight: f32,
}

/// Raw edge list as read from DuckDB, before IDF attenuation and stochastic normalization.
/// Kept separate so the transition matrix can be rebuilt with different balancing parameters
/// without re-reading the database.
#[derive(Debug, Clone, Default)]
pub struct RawEdges {
    pub node_to_id: HashMap<String, u32>,
    pub id_to_node: Vec<String>,
    pub relation_types: Vec<String>,
    rel_to_id: HashMap<String, u8>,
    /// Symmetric edge tuples for diffusion (src, dst, weight)
    raw_edges: Vec<(u32, u32, f32)>,
    /// Original directed tuples (src, dst, rel_id, weight)
    directed_edges: Vec<(u32, u32, u8, f32)>,
    seen_directed: HashSet<(u32, u32, u8)>,
    in_degrees: Vec<u32>,
}

impl RawEdges {
    fn node_id(&mut self, key: &str) -> u32 {
        if let Some(&id) = self.node_to_id.get(key) {
            return id;
        }
        let id = self.id_to_node.len() as u32;
        self.node_to_id.insert(key.to_string(), id);
        self.id_to_node.push(key.to_string());
        id
    }

    fn rel_id(&mut self, rel: &str) -> u8 {
        if let Some(&id) = self.rel_to_id.get(rel) {
            return id;
        }
        let id = self.relation_types.len() as u8;
        self.rel_to_id.insert(rel.to_string(), id);
        self.relation_types.push(rel.to_string());
        id
    }

    /// Adds a directed semantic edge and its symmetric diffusion counterpart.
    pub fn push(&mut self, src_key: &str, dst_key: &str, rel: &str, w: f32) {
        let u = self.node_id(src_key);
        let v = self.node_id(dst_key);
        let rel_id = self.rel_id(rel);

        let max_idx = u.max(v) as usize;
        if self.in_degrees.len() <= max_idx {
            self.in_degrees.resize(max_idx + 1, 0);
        }

        if !self.seen_directed.insert((u, v, rel_id)) {
            return;
        }

        self.directed_edges.push((u, v, rel_id, w));

        self.in_degrees[v as usize] += 1;
        self.raw_edges.push((u, v, w));

        if u != v {
            self.in_degrees[u as usize] += 1;
            self.raw_edges.push((v, u, w));
        }
    }

    pub fn num_nodes(&self) -> usize {
        self.id_to_node.len()
    }

    /// Reads semantic edges from DuckDB (read-only).
    pub fn load_from_duckdb(db_path: &str, query_edges_sql: Option<&str>) -> Result<Self> {
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

        const COUNTRY_KEYWORD_IDS: &[&str] = &[
            // Country names
            "keyword:300",   // uk
            "keyword:301",   // united kingdom
            "keyword:519",   // china
            "keyword:529",   // germany
            "keyword:557",   // portugal
            "keyword:647",   // russia
            "keyword:652",   // france
            "keyword:676",   // scotland
            "keyword:700",   // sweden
            "keyword:974",   // brazil
            "keyword:978",   // spain
            "keyword:1098",  // japan
            "keyword:1207",  // ireland
            "keyword:1236",  // denmark
            "keyword:1259",  // united states
            "keyword:1370",  // canada
            "keyword:1473",  // australia
            "keyword:1489",  // italy
            "keyword:2043",  // mexico
            "keyword:13402", // norway
            "keyword:15288", // brasil
            "keyword:19064", // usa
            "keyword:19557", // korea
            "keyword:19654", // south korea
            "keyword:25010", // us
            "keyword:25583", // finland
            "keyword:36572", // argentina
            "keyword:42169", // colombia
            "keyword:42224", // chile
            // Demonyms and Language tags
            "keyword:258",   // german
            "keyword:271",   // american
            "keyword:277",   // italian
            "keyword:299",   // british
            "keyword:330",   // french
            "keyword:361",   // english
            "keyword:958",   // danish
            "keyword:1702",  // canadian
            "keyword:1843",  // scottish
            "keyword:13177", // swedish
            "keyword:13329", // finnish
            "keyword:13496", // irish
            "keyword:18970", // australian
            "keyword:18977", // brazilian
            "keyword:19568", // korean
            "keyword:24996", // japanese
            "keyword:25198", // portuguese
            "keyword:30617", // spanish
            "keyword:30714", // norwegian
            "keyword:36364", // chinese
            "keyword:42125", // russian
        ];

        let mut raw = RawEdges::default();

        while let Some(row) = rows.next()? {
            let src_key: String = row.get(0)?;
            let dst_key: String = row.get(1)?;
            let mut base_w: f64 = row.get(2)?;
            let rel_str: String = row.get(3).unwrap_or_else(|_| "connected".to_string());

            // Geographic, nationality and language tags are broad metadata, NOT aesthetic genres.
            // Heavily attenuate them (by 98%) so diffusion does not leak between completely
            // unrelated genres (e.g. hip-hop to grunge, or trap to samba) merely because
            // both works share 'american', 'us', 'english' or 'brazilian'.
            let is_country_kw = COUNTRY_KEYWORD_IDS.contains(&src_key.as_str())
                || COUNTRY_KEYWORD_IDS.contains(&dst_key.as_str());
            if is_country_kw {
                base_w *= 0.02;
            }

            raw.push(&src_key, &dst_key, &rel_str, base_w as f32);
        }

        if raw.num_nodes() == 0 {
            anyhow::bail!("No edges found in DuckDB query.");
        }

        Ok(raw)
    }
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

    /// Inter-domain balancing exponent used to build `weights` (1.0 = legacy, 0.0 = equal split).
    pub domain_gamma: f32,
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
    /// inter-domain balancing (gamma from `YML_DOMAIN_GAMMA`), and the directed CSR index.
    pub fn load_from_duckdb(db_path: &str, query_edges_sql: Option<&str>) -> Result<Self> {
        let raw = RawEdges::load_from_duckdb(db_path, query_edges_sql)?;
        Ok(Self::from_raw(raw, domain_gamma_from_env()))
    }

    /// Builds the stochastic transition matrix from raw edges.
    ///
    /// Inter-domain normalization: for every source node `u`, the total (IDF-attenuated) mass that
    /// flows toward item nodes is preserved, but it is re-split across item domains proportionally to
    /// `S_d^gamma`, where `S_d` is the mass `u` would send to domain `d` under legacy weighting.
    /// Within a domain, the relative proportions between neighbors are unchanged, and mass toward
    /// non-item nodes (keywords, people, studios, epochs) is untouched.
    /// - `gamma = 1.0`: legacy behavior.
    /// - `gamma = 0.0`: each item domain present in `u`'s neighborhood receives an equal share.
    pub fn from_raw(raw: RawEdges, gamma: f32) -> Self {
        let gamma = gamma.clamp(0.0, 1.0);
        let RawEdges {
            node_to_id,
            id_to_node,
            relation_types,
            raw_edges,
            directed_edges,
            in_degrees,
            ..
        } = raw;

        let num_nodes = id_to_node.len();
        let num_edges = raw_edges.len();

        // 1. Applies in-memory Adamic-Adar / IDF attenuation with square-root penalty for large hubs:
        // prevents supernode leaking across domains.
        let mut adjusted_edges: Vec<(u32, u32, f32)> = Vec::with_capacity(num_edges);
        for (u, v, base_w) in raw_edges {
            let deg = in_degrees[v as usize].max(1) as f32;
            let idf = if deg > 80.0 {
                1.0f32 / deg.sqrt()
            } else {
                1.0f32 / (1.0f32 + deg).ln().max(1.0f32)
            };
            adjusted_edges.push((u, v, base_w * idf));
        }

        // 2. Inter-domain normalization of outgoing mass toward item domains.
        if gamma < 1.0 {
            let node_domain: Vec<u8> = id_to_node
                .iter()
                .map(|k| item_domain_of(k).map(|d| d as u8).unwrap_or(NO_DOMAIN))
                .collect();

            let mut dom_sums: Vec<[f32; NUM_ITEM_DOMAINS]> =
                vec![[0.0; NUM_ITEM_DOMAINS]; num_nodes];
            for &(u, v, w) in &adjusted_edges {
                let d = node_domain[v as usize];
                if d != NO_DOMAIN {
                    dom_sums[u as usize][d as usize] += w;
                }
            }

            // Per-node scale factor for each item domain
            let mut scales: Vec<[f32; NUM_ITEM_DOMAINS]> = vec![[1.0; NUM_ITEM_DOMAINS]; num_nodes];
            for (u, sums) in dom_sums.iter().enumerate() {
                let present = sums.iter().filter(|&&s| s > 0.0).count();
                if present < 2 {
                    continue;
                }
                let total: f32 = sums.iter().sum();
                let tempered: [f32; NUM_ITEM_DOMAINS] = std::array::from_fn(|d| {
                    if sums[d] > 0.0 {
                        sums[d].powf(gamma)
                    } else {
                        0.0
                    }
                });
                let tempered_total: f32 = tempered.iter().sum();
                for d in 0..NUM_ITEM_DOMAINS {
                    if sums[d] > 0.0 {
                        let target = total * tempered[d] / tempered_total;
                        scales[u][d] = target / sums[d];
                    }
                }
            }

            for (u, v, w) in adjusted_edges.iter_mut() {
                let d = node_domain[*v as usize];
                if d != NO_DOMAIN {
                    *w *= scales[*u as usize][d as usize];
                }
            }
        }

        // 3. Normalizes to stochastic Markov transition probabilities: P[u, v]
        // and groups by destination node v (CSC format - in-edges)
        let mut out_sums: Vec<f32> = vec![0.0f32; num_nodes];
        for &(u, _, w) in &adjusted_edges {
            out_sums[u as usize] += w;
        }

        let mut in_adjacency: Vec<Vec<(u32, f32)>> = vec![Vec::new(); num_nodes];
        for (u, v, eff_w) in adjusted_edges {
            let sum_u = out_sums[u as usize];
            let prob = if sum_u > 0.0 { eff_w / sum_u } else { 0.0 };
            in_adjacency[v as usize].push((u, prob));
        }

        // 4. Packs into contiguous, cache-friendly CSC structures for PPR
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

        // 5. Builds CSR index of original directed semantic edges by source node
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

        Self {
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
            domain_gamma: gamma,
        }
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

    /// Transition probability P[src, dst] in the diffusion matrix (0.0 if no edge).
    pub fn transition_prob(&self, src_key: &str, dst_key: &str) -> f32 {
        let (Some(u), Some(v)) = (self.get_node_id(src_key), self.get_node_id(dst_key)) else {
            return 0.0;
        };
        let v = v as usize;
        (self.indptr[v]..self.indptr[v + 1])
            .filter(|&i| self.indices[i] == u)
            .map(|i| self.weights[i])
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One keyword shared by 1 movie and 10 artists, plus a person linked to the movie.
    fn skewed_raw() -> RawEdges {
        let mut raw = RawEdges::default();
        raw.push("movie:1", "keyword:dark", "has_keyword", 1.0);
        for i in 0..10 {
            raw.push(&format!("artist:{i}"), "keyword:dark", "has_tag", 1.0);
        }
        raw.push("movie:1", "person:1", "has_director", 1.0);
        raw
    }

    fn row_sums(g: &GraphCSR) -> Vec<f32> {
        let mut sums = vec![0.0f32; g.num_nodes];
        for v in 0..g.num_nodes {
            for i in g.indptr[v]..g.indptr[v + 1] {
                sums[g.indices[i] as usize] += g.weights[i];
            }
        }
        sums
    }

    fn mass_to_domain(g: &GraphCSR, src: &str, domain: usize) -> f32 {
        g.id_to_node
            .iter()
            .filter(|k| item_domain_of(k) == Some(domain))
            .map(|k| g.transition_prob(src, k))
            .sum()
    }

    #[test]
    fn rows_are_stochastic_for_all_gammas() {
        for gamma in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let g = GraphCSR::from_raw(skewed_raw(), gamma);
            for (u, s) in row_sums(&g).iter().enumerate() {
                assert!(
                    (s - 1.0).abs() < 1e-5,
                    "gamma={gamma} node={} sum={s}",
                    g.id_to_node[u]
                );
            }
        }
    }

    #[test]
    fn gamma_one_matches_legacy_split() {
        let g = GraphCSR::from_raw(skewed_raw(), 1.0);
        // Legacy: keyword splits by IDF only. movie:1 has degree 2 (keyword + person) -> 1/ln(3);
        // each artist has degree 1 -> 1/max(ln(2), 1) = 1.
        let idf_movie = 1.0f32 / 3.0f32.ln();
        let expected_movie = idf_movie / (idf_movie + 10.0);
        let p_movie = mass_to_domain(&g, "keyword:dark", 0);
        let p_music = mass_to_domain(&g, "keyword:dark", 4);
        assert!((p_movie - expected_movie).abs() < 1e-5, "movie={p_movie}");
        assert!(
            (p_music - (1.0 - expected_movie)).abs() < 1e-5,
            "music={p_music}"
        );
    }

    #[test]
    fn gamma_zero_splits_equally_across_domains() {
        let g = GraphCSR::from_raw(skewed_raw(), 0.0);
        let p_movie = mass_to_domain(&g, "keyword:dark", 0);
        let p_music = mass_to_domain(&g, "keyword:dark", 4);
        assert!((p_movie - 0.5).abs() < 1e-5, "movie={p_movie}");
        assert!((p_music - 0.5).abs() < 1e-5, "music={p_music}");
        // Within-domain proportions are preserved: each artist gets 1/10 of music mass.
        let p_a0 = g.transition_prob("keyword:dark", "artist:0");
        assert!((p_a0 - 0.05).abs() < 1e-5);
    }

    #[test]
    fn intermediate_gamma_is_monotonic() {
        let shares: Vec<f32> = [1.0, 0.75, 0.5, 0.25, 0.0]
            .iter()
            .map(|&g| mass_to_domain(&GraphCSR::from_raw(skewed_raw(), g), "keyword:dark", 0))
            .collect();
        for w in shares.windows(2) {
            assert!(
                w[1] > w[0],
                "minority share must grow as gamma decreases: {shares:?}"
            );
        }
    }

    #[test]
    fn non_item_mass_is_preserved() {
        let legacy = GraphCSR::from_raw(skewed_raw(), 1.0);
        let balanced = GraphCSR::from_raw(skewed_raw(), 0.0);
        // movie:1 -> {keyword, person}: no item neighbors, so nothing changes.
        for dst in ["keyword:dark", "person:1"] {
            let a = legacy.transition_prob("movie:1", dst);
            let b = balanced.transition_prob("movie:1", dst);
            assert!((a - b).abs() < 1e-6, "{dst}: {a} vs {b}");
        }
    }
}
