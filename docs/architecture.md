# System Architecture and Graph Model

`youmightlike` operates on a foundational engineering premise: **modern columnar relational databases (DuckDB) serve as exceptional in-process feature stores for feeding sparse linear algebra and graph diffusion algorithms on CPU**.

Instead of maintaining cumbersome external graph databases (e.g. Neo4j) or relying on costly, opaque LLM embedding APIs, the system structures domain knowledge in typed dimensional tables and projects the cultural graph dynamically via high-performance SQL views.

---

## 1. Architectural Topology

```
┌─────────────────────────────────────────────────────────────┐
│                        DATA SOURCES                         │
│   IMDb Dumps   │   Steam Parquet   │  Goodreads  │  Music   │
└──────────────────────────────┬──────────────────────────────┘
                               │
                       DuckDB Ingestion
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                 DUCKDB (Local Feature Store)                │
│  - Canonical Tables: movies, people, games, books, etc.     │
│  - Central Vocabulary Table: keywords                       │
│                                                             │
│  PROJECTION VIEWS:                                          │
│    ├─ v_graph_nodes           (Unified nodes: 357k)         │
│    ├─ v_graph_edges           (Active edges: 1.95M directed)│
│    └─ v_weighted_graph_edges  (Symmetric CSR: 3.91M trans)  │
└──────────────────────────────┬──────────────────────────────┘
                               │
                     Zero-Copy / Memory Scan
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                  STATISTICAL DIFFUSION CORE                 │
│             (Personalized PageRank / Random Walk)           │
│                                                             │
│  - User Active Seeds: [Movie X, Album Y, Game Z]            │
│  - Sparse Transition Matrix: A[u, v] with IDF weights       │
│  - Power Iteration: p_{k+1} = α · P^T · p_k + (1-α)·s       │
│  - Explainability: Path backtracking along maximum flux     │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Relational Schema and Transmedia Bridges

Canonical tables store entity records for each cultural domain. Transmedia bridges are formed by:

1. **People and Soundtrack Composers (`movie_credits` ➔ `people` ➔ `music_artists`):**
   Film composers (e.g., Vangelis, Trent Reznor, Hans Zimmer, Ennio Morricone, Joe Hisaishi) connect cinema directly to their discographies and peer artists via `is_artist` and `has_composer`.
2. **Literary Adaptations (`movies.based_on_wikidata_id` ➔ `books`):**
   Films and television series based on novels form the `based_on` / `adapted_as` cross-domain edge.
3. **Unified Aesthetic Vocabulary (`keywords`):**
   Cinema genres, Steam community tags, literary tropes, and musical microgenres converge into the unified `keywords` table. Thus, a sci-fi film, a cyberpunk video game, a dystopian novel, and a dark synthwave album share identical thematic anchor nodes.

---

## 3. Edge Weighting and Hub Mitigation (Adamic-Adar / IDF)

If all connections shared identical edge weights, ultra-generic tags (such as *"Action"*, *"Rock"*, *"Fiction"*) would act as probability sinks in the graph, biasing PageRank toward obvious mainstream blockbusters.

To solve this deterministically, the `v_weighted_graph_edges` view attenuates edge weights using the inverse logarithm of the target node's in-degree:

$$w_{\text{effective}}(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{degree}(v))}$$

* Rare, specific connections (e.g. niche tag *"post-cyberpunk"*, cult author, specific soundtrack composer) retain high energy transfer.
* Broad, ubiquitous connections (e.g. *"drama"*, *"music"*) have their transition probability penalized proportionally to their saturation across the graph.

---

## 4. Projection Views in DuckDB

The entire graph is projected dynamically over the underlying tables:

* **`v_graph_nodes`:** Unified projection with compound keys in `<type>:<id>` format (e.g. `movie:183`, `artist:5490`, `book:12401`), entity display name, node type, and cultural domain (`movie`, `game`, `book`, `music`, `concept`).
* **`v_graph_edges`:** Union of all oriented relations across the system (`has_director`, `has_composer`, `is_artist`, `released`, `written_by`, `has_subject`, `has_tag`, `has_keyword`, `developed_by`, `similar_to`, `soundtrack_track`, `based_on`).
* **`v_weighted_graph_edges`:** Dynamically calculates node degrees and produces `(source_id, target_id, effective_weight)` ready to be loaded into Compressed Sparse Row (CSR) / Compressed Sparse Column (CSC) matrix structures in Rust.

---

## 5. Current Scale and Engine Details

For an in-depth breakdown of bidirectional diffusion in Rust, structural seed anchors, soundtrack link tables, and the zero-freeze HTML5 Canvas force rendering engine, see [System Evolution (November 2026)](./system_evolution_2026.md).

* **Active Semantic Edges:** 1,956,347 active edges in DuckDB (`graph_semantic_edges`), expanding to 3.91M directed transitions with symmetric diffusion mirroring.
* **Catalog Coverage:** >34k films, >20k games, >41k books, >3.25M artists, and >40.4M tracks.
* **Diffusion Engine:** `youmightlike-engine` in Rust (bidirectional Personalized PageRank over CSR matrix parallelized with Rayon and hosted via Axum).
* **Public Feature Store:** The complete dataset (2.29 GB Parquet ZSTD) is published on Hugging Face: [**`lnvck/youmightlike-dataset`**](https://huggingface.co/datasets/lnvck/youmightlike-dataset).
* **Lite Deployment Profile:** `scripts/export_lite_db.py` creates a 140 MB DuckDB file (`taste_graph_lite.duckdb`) preserving 100% of cinema, anime, TV, games, books, and connected artists for low-memory production hosting (GCP Always Free / Cloud Run).

For academic foundations and theoretical lineage (Pixie, Topic-Sensitive PageRank, Adamic-Adar, PathSim), see [Academic Foundations & Prior Art](./references.md).


