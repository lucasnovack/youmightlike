# youmightlike

Transmedia cultural recommendation engine connecting **Cinema, TV Series, Music, Literature, and Video Games** into a unified aesthetic affinity graph.

Instead of isolated domain silos, `youmightlike` projects deep cultural relationships—soundtrack compositions, literary adaptations, studio lineages, microgenres, co-listening patterns, and conceptual tropes—onto a high-density graph in DuckDB, computing personalized cross-media recommendations via bidirectional stochastic diffusion (Personalized PageRank) powered by a native Rust engine.

---

## Key Features

* **Bipolar Diffusion with Negative Feedback (Anti-Seeds):** Damps unwanted aesthetics using vector subtraction $p_{\text{final}} = \max(0.0, p_{\text{pos}} - \beta \cdot p_{\text{neg}})$ executed in parallel across CPU cores with Rayon. Negative seeds render with distinct crimson repulsion halos on the canvas.
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
Directed semantic edges $(u, v)$ (*e.g., movie $\to$ keyword*) receive symmetric pairs $(v, u)$ during Compressed Sparse Row (CSR) matrix construction. This allows Markovian energy to flow from a seed work into thematic concepts and disperse back into kindred albums, games, and novels.

### 3. Inter-Domain Balancing ($\gamma$)
Inter-domain balancing exponent (`YML_DOMAIN_GAMMA`, $\gamma \in [0.0, 1.0]$) prevents catalog mass disparity (e.g. 40M music tracks vs 20k games) from skewing multi-media walks. $\gamma = 0.0$ splits transition probability equally across item domains, while $\gamma = 1.0$ preserves natural degrees.

### 4. Bipolar Repulsive Damping
Negative seeds inject a repulsive vector subtracted from positive energy:

$$p_{\text{final}}(v) = \max\left(0.0,\, p_{\text{pos}}(v) - \beta \cdot p_{\text{neg}}(v)\right)$$

Where $\beta$ (default $1.2$) tunes repulsion strength, zeroing out aversive clusters without negative probability artifacts.

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
| **Active Semantic Edges** (`graph_semantic_edges`) | **3,818,452** | Filtered, cleaned, and IDF-attenuated edges |

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

## License

MIT License. External datasets adhere to their respective open licenses (CC0 1.0, ODC-By, and non-commercial community open datasets).
