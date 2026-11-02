use anyhow::{Context, Result};
use duckdb::{AccessMode, Config, Connection};
use std::collections::HashMap;

/// Grafo esparso otimizado em memória no formato CSC (Compressed Sparse Column)
/// para permitir cálculo do Personalized PageRank 100% paralelo e lock-free via Rayon.
pub struct GraphCSR {
    pub num_nodes: usize,
    pub num_edges: usize,
    /// Apontadores de início de cada nó no vetor de índices (tamanho: num_nodes + 1)
    pub indptr: Vec<usize>,
    /// Índices dos nós de origem para cada aresta entrante (tamanho: num_edges)
    pub indices: Vec<u32>,
    /// Pesos de transição estocásticos normalizados (Markov: sum(P[u, :]) == 1.0)
    pub weights: Vec<f32>,
    /// Mapeamento de chave de texto (ex: 'artist:11', 'track:38862') para ID numérico
    pub node_to_id: HashMap<String, u32>,
    /// Mapeamento inverso de ID numérico para chave de texto
    pub id_to_node: Vec<String>,
}

impl GraphCSR {
    /// Carrega as arestas do DuckDB e constrói o CSC em memória com atenuação Adamic-Adar / IDF.
    pub fn load_from_duckdb(
        db_path: &str,
        query_edges_sql: Option<&str>,
    ) -> Result<Self> {
        let conn = Connection::open_with_flags(
            db_path,
            Config::default().access_mode(AccessMode::ReadOnly)?,
        )
        .context("Falha ao abrir banco DuckDB em modo read_only")?;

        conn.execute_batch("PRAGMA threads=8; PRAGMA memory_limit='16GB';")?;

        let sql = query_edges_sql.unwrap_or(
            "SELECT source_id, target_id, base_weight FROM v_graph_edges WHERE base_weight > 0.0"
        );

        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query([])?;

        let mut node_to_id: HashMap<String, u32> = HashMap::new();
        let mut id_to_node: Vec<String> = Vec::new();

        let mut get_or_create_id = |key: &str, node_map: &mut HashMap<String, u32>, id_list: &mut Vec<String>| -> u32 {
            if let Some(&id) = node_map.get(key) {
                id
            } else {
                let id = id_list.len() as u32;
                node_map.insert(key.to_string(), id);
                id_list.push(key.to_string());
                id
            }
        };

        // Tuplas brutas de arestas (src, dst, weight)
        let mut raw_edges: Vec<(u32, u32, f32)> = Vec::new();
        let mut in_degrees: HashMap<u32, u32> = HashMap::new();

        while let Some(row) = rows.next()? {
            let src_key: String = row.get(0)?;
            let dst_key: String = row.get(1)?;
            let base_w: f64 = row.get(2)?;

            let u = get_or_create_id(&src_key, &mut node_to_id, &mut id_to_node);
            let v = get_or_create_id(&dst_key, &mut node_to_id, &mut id_to_node);

            *in_degrees.entry(v).or_insert(0) += 1;
            raw_edges.push((u, v, base_w as f32));
        }

        let num_nodes = id_to_node.len();
        let num_edges = raw_edges.len();

        if num_nodes == 0 {
            anyhow::bail!("Nenhuma aresta encontrada na consulta ao DuckDB.");
        }

        // 1. Aplica atenuação Adamic-Adar / IDF in-memory: w = base_w / ln(1 + degree(v))
        // e calcula os graus de saída de cada nó u para normalização estocástica
        let mut out_sums: Vec<f32> = vec![0.0f32; num_nodes];
        let mut adjusted_edges: Vec<(u32, u32, f32)> = Vec::with_capacity(num_edges);

        for (u, v, base_w) in raw_edges {
            let deg = *in_degrees.get(&v).unwrap_or(&1) as f32;
            let idf = 1.0f32 / (1.0f32 + deg).ln().max(1.0f32);
            let eff_w = base_w * idf;
            out_sums[u as usize] += eff_w;
            adjusted_edges.push((u, v, eff_w));
        }

        // 2. Normaliza para probabilidades estocásticas de transição de Markov: P[u, v]
        // E agrupa por nó de destino v (formato CSC - in-edges)
        let mut in_adjacency: Vec<Vec<(u32, f32)>> = vec![Vec::new(); num_nodes];
        for (u, v, eff_w) in adjusted_edges {
            let sum_u = out_sums[u as usize];
            let prob = if sum_u > 0.0 { eff_w / sum_u } else { 0.0 };
            in_adjacency[v as usize].push((u, prob));
        }

        // 3. Empacota nas estruturas contíguas e compactas de CSR/CSC
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

        Ok(Self {
            num_nodes,
            num_edges,
            indptr,
            indices,
            weights,
            node_to_id,
            id_to_node,
        })
    }

    /// Retorna o ID numérico correspondente a uma chave de nó (ex: 'artist:11')
    pub fn get_node_id(&self, key: &str) -> Option<u32> {
        self.node_to_id.get(key).copied()
    }

    /// Retorna a chave textual de um nó dado seu ID numérico
    pub fn get_node_key(&self, id: u32) -> Option<&str> {
        self.id_to_node.get(id as usize).map(|s| s.as_str())
    }
}
