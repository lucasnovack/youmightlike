# youmightlike

Transmedia cultural recommendation engine connecting **Cinema, TV Series, Music, Literature, and Video Games** into a unified aesthetic affinity graph.

Instead of isolated domain silos, `youmightlike` projects deep cultural relationships—soundtrack compositions, literary adaptations, studio lineages, microgenres, co-listening patterns, and conceptual tropes—onto a high-density graph in DuckDB, computing personalized cross-media recommendations via bidirectional stochastic diffusion (Personalized PageRank) powered by a native Rust engine.

---

## Key Features

* **Pixie-Style Multi-Hit Booster:** When multiple seeds $Q$ are provided, energy is non-linearly aggregated as $p_{\text{multi}}(v) = (\sum_{q \in Q} \sqrt{w_q \cdot p_q(v)})^2$, conferring quadratic reinforcement to candidates at the intersection of diverse taste anchors.
* **Bipolar Diffusion with Negative Feedback (Anti-Seeds):** Damps unwanted aesthetics using vector subtraction `p_final = max(0.0, p_pos - β · p_neg)` executed in parallel across CPU cores with Rayon. Negative seeds render with distinct crimson repulsion halos on the canvas.
* **Cultural Ticket (Receiptify / Aesthetic Snapshot):** Generates high-DPI (2x) thermal receipts on HTML5 `<canvas>` with perforated saw-tooth tear, functional taste hash barcode, orbital constellation mini-map, and unconsumed recommendations ready for PNG export or clipboard copy.
* **"Already Know" & Serendipity Filter:** Neutral discard action (`[Already Know]`) that stores consumed works locally (`known_items`) and frees recommendation quota for undiscovered works without negatively warping taste diffusion.
* **Explainable Behavioral Bridges:** Explicit contextual resonance labels: Last.fm listener co-audition overlap (`match_score`), Steam co-play similarity ("Players of X also play Y"), cross-domain literary adaptations, and composed original soundtracks.
* **Dual-Mode Sidebar Panel:**
  * **Discoveries:** Categorized feed by media domain with instant client-side filtering and sorting.
  * **Live Inspection:** Real-time multi-node neighborhood expansion with mutual connection counts and clickable `#tag +` seed pills.
* **Local-First & Offline PWA:** Service Worker (`sw.js`) with synthetic POST payload caching, responsive connection badge, and zero cloud runtime API dependencies.

---

## Mathematical & Topological Foundations

### 1. Edge Weighting (Adamic-Adar / IDF)
To prevent ubiquitous categories (*Action*, *Rock*, *Drama*) from acting as probability sinks and biasing suggestions toward mainstream blockbusters, edge weights are attenuated using the inverse log of the target node's in-degree:

$$\text{weight}(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{degree}(v))}$$

Rare, specific connections (e.g. niche tag *"post-cyberpunk"*, auteur director, or specific soundtrack composer) preserve high probability transfer.

### 2. Bidirectional Symmetric Diffusion
Directed semantic edges `(u, v)` (*e.g., movie → keyword*) receive symmetric pairs `(v, u)` during Compressed Sparse Row (CSR) matrix construction. This allows Markovian energy to flow from a seed work into thematic concepts and disperse back into kindred albums, games, and novels.

### 3. Inter-Domain Balancing (γ)
Inter-domain balancing exponent (`YML_DOMAIN_GAMMA`, `γ ∈ [0.0, 1.0]`) prevents catalog mass disparity (e.g. 40M music tracks vs 20k games) from skewing multi-media walks. `γ = 0.0` splits transition probability equally across item domains, while `γ = 1.0` preserves natural degrees.

### 4. Bipolar Repulsive Damping
Negative seeds inject a repulsive vector subtracted from positive energy:

$$p_{\text{final}}(v) = \max\left(0.0,\, p_{\text{pos}}(v) - \beta \cdot p_{\text{neg}}(v)\right)$$

Where `β` (default 1.2) tunes repulsion strength, zeroing out aversive clusters without negative probability artifacts.

---

## Current Graph Scale (DuckDB Feature Store)

The complete graph is stored and queried locally in `data/processed/taste_graph.duckdb`:

| Domain / Dimension | Catalog Records | Semantic Connections & Source |
| :--- | :---: | :--- |
| **Movies & Anime** (`movies`) | **34,188** | IMDb Datasets + MyAnimeList Top Anime |
| **TV Series & Mini-Series** (`tv_series`) | **7,647** | IMDb Datasets (`tvSeries`, `tvMiniSeries`) |
| **People & Studios** (`people`) | **37,498** | Directors, writers, and animation studios |
| **Credits & Production** (`movie_credits`) | **106,423** | Direction, writing, and studios (`produced_by`) |
| **Movie/Anime Keywords** (`movie_keywords`) | **239,528** | MovieLens Tag Genome + MyAnimeList tropes |
| **Video Games** (`games`) | **20,064** | Steam Dataset + Console Classics (Nintendo/PS/Sega) |
| **Game Developers** (`game_developers`) | **9,250** | PC and console studios & publishers |
| **Game Tags** (`game_tags`) | **133,983** | Steam thematic tags + platform identifiers |
| **Game Co-Play Similarities** (`game_similarities`) | **20,854** | Vectorized user playtime cosine similarity |
| **Books** (`books`) | **38,203** | Goodreads 100k (CC0 Parquet) |
| **Authors** (`book_authors`) | **29,742** | Canonical authors and co-authors |
| **Literary Themes** (`book_subjects`) | **400,285** | Goodreads genres and subjects |
| **Cross-Media Adaptations** (`based_on`) | **11,010** | Wikidata SPARQL film/series-to-book links |
| **Musical Artists** (`music_artists`) | **3,257,918** | MusicBrainz + Spotify Mass Ingestion |
| **Music Albums** (`music_albums`) | **18,034,542** | Discographies and studio releases |
| **Music Tracks** (`music_tracks`) | **40,418,286** | Audio features (BPM, Valence, Energy, Danceability) |
| **Artist Similarities** (`artist_similarities`) | **61,680** | Last.fm Co-listening Graph (HetRec) |
| **Active Semantic Edges** (`graph_semantic_edges`) | **1,956,347** | Filtered, cleaned, and IDF-attenuated edges |

> [!TIP]
> **Open Dataset on Hugging Face:** The complete feature store and diffusion graph (2.29 GB in Parquet ZSTD, including all 40.4M tracks with audio features) is publicly hosted at [**`lnvck/youmightlike-dataset`**](https://huggingface.co/datasets/lnvck/youmightlike-dataset).
>
> **Lite Production Profile (`taste_graph_lite.duckdb`):** `scripts/export_lite_db.py` creates a 140 MB DuckDB file preserving 100% of cinema, anime, TV, games, books, and active connected music nodes:
> * **Unique Connected Nodes:** **357,643**
> * **Persisted Directed Edges:** **1,956,347**
> * **In-Memory Stochastic Transitions (`GraphCSR`):** **3,912,424** (bidirectional symmetric expansion for PPR diffusion, discounting self-loops)
> * **Curated Output Quota:** Calibrated ~24–28 recommendations (4–5 per domain) to prevent visual overload while ensuring cross-domain breadth.
> * **Printable Cultural Passport:** 3x Retina HTML5 Canvas export (`dpr = 3`), deterministic Code 128 barcode, Apple Passbook squircle emblem, and perforated tear notches.

---

## Repository Structure

```
youmightlike/
├── crates/
│   └── engine/             # High-performance Rust diffusion core (Axum, Rayon, CSR)
│       ├── src/
│       │   ├── graph.rs            # CSR matrix, edge mirroring, gamma balancing
│       │   ├── ppr.rs              # Parallel Personalized PageRank & negative damping
│       │   ├── recommendation/     # Modular domain extractors & graph builder
│       │   ├── search.rs           # In-RAM CatalogCache search & autocomplete
│       │   ├── service.rs          # Axum routing & shared AppState
│       │   └── bin/server.rs       # Native server binary (`youmightlike-server`)
├── python/
│   ├── src/youmightlike/
│   │   ├── connectors/     # Live fetchers (Wikidata SPARQL, MusicBrainz, OpenLibrary)
│   │   ├── storage/        # Schema definitions and batch ingestion scripts
│   │   └── web/            # HTML5 Canvas constellation UI & PWA assets
│   └── tests/              # Contract and serialization tests (pytest)
├── data/                   # Local DuckDB database & raw dumps (git-ignored)
└── docs/                   # Architecture, data sources, and system evolution logs
```

---

## Quickstart & Running Locally

### 1. Prerequisites
* **Rust:** Stable toolchain (`rustup`)
* **Python:** 3.11+ with `venv`

### 2. Environment Setup
```bash
# Set up Python virtual environment
python3 -m venv .venv
source .venv/bin/activate

# Install Python package with dev tools
pip install -e "python/[dev]"

# Download precompiled DuckDB shared library (~30MB) for instant sub-second builds
./scripts/setup_duckdb.sh
```

### 3. Launching the Native Web Server
The HTTP server and CSR diffusion engine run natively in Rust (`youmightlike-server` via Tokio and Axum), keeping the semantic graph in memory for sub-millisecond responses:

```bash
# Build release binaries
cargo build --release

# Run Rust server on port 8000
./target/release/youmightlike-server --port 8000
```

Open `http://localhost:8000` in your browser:
* **Add Seeds:** Search across movies, series, games, books, artists, or tracks (`[+ Like]` or `[− Avoid]`).
* **Interactive Constellation:** Explore 3-ring orbital graph rendered in HTML5 Canvas.
* **Multi-Node Selection:** `Shift + Click` nodes to inspect mutual connection paths and bridge origins.
* **Thermal Receipt:** Click `[Receiptify]` in the header to preview and export your printable cultural ticket.

---

## Testing & Quality Assurance

Both the Rust engine and Python data layers feature dedicated test suites:

```bash
# Run Rust engine tests (PPR stochastic convergence, zero-orphan topology, bilingual metadata)
cargo test

# Run Rust linter
cargo clippy --all-targets -- -D warnings

# Run Python contract tests
pytest python/tests

# Run Python linter
ruff check python/
```

---

## References & Academic Foundations

`youmightlike` builds upon seminal concepts in graph random walks, link prediction, and sparse matrix linear algebra:

* **[Pixie (Pinterest, WWW '18)](docs/references.md#1-real-time-graph-walks--multi-seed-diffusion):** Real-time random walk recommendation on object graphs, multi-hit booster aggregation, and degree-scaled query allocation.
* **[Personalized PageRank (Stanford, WWW '02 / '03)](docs/references.md#2-stochastic-diffusion--personalized-pagerank):** Topic-sensitive teleport vectors and stationary probability distributions over directed networks.
* **[Adamic-Adar Index (Social Networks '03)](docs/references.md#3-link-weighting--hub-attenuation):** Inverse-log in-degree weighting to suppress hub sinks and amplify niche semantic conductance.
* **[PathSim & Heterogeneous Networks (VLDB '11)](docs/references.md#4-heterogeneous-information-networks-hin):** Meta-path discovery across transmedia adaptations, original soundtracks, and co-listening graphs.
* **[TrustRank & Bipolar Walks (VLDB '04)](docs/references.md#5-repulsive-random-walks--signed-graphs):** Dual-diffusion signed damping with repulsive negative vectors.

See the complete bibliography and mathematical mapping in [docs/references.md](docs/references.md).

---

## License

MIT License. External datasets adhere to their respective open licenses (CC0 1.0, ODC-By, and non-commercial community open datasets).
