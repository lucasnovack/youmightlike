# youmightlike

Transmedia recommendation system connecting **Cinema, TV Series, Music, Literature, and Video Games** into a unified aesthetic affinity graph.

Instead of isolated domain silos, `youmightlike` projects cultural relationships (directing, original soundtracks, source literature, studios, microgenres, and conceptual tags) onto a semantic graph in DuckDB, computing recommendations via statistical diffusion (Personalized PageRank) powered by a native Rust engine.

---

## Technical Principles

* **Explainability:** Every recommendation maintains a traceable path in the graph (*e.g., Movie A connected to Artist B via Composer C and shared Cyberpunk aesthetic*).
* **Computational Efficiency:** Sparse CPU linear algebra via CSR matrices parallelized with Rayon, executing in sub-second time without external services.
* **Privacy and Autonomy:** Local columnar storage in DuckDB with zero runtime inference APIs or telemetry.

---

## Architecture

```
[ Open Datasets (IMDb, Steam, Goodreads, MusicBrainz, Spotify) ]
                               │
                    Columnar Ingestion (Parquet/TSV)
                               ▼
               ┌──────────────────────────────────┐
               │    Local DuckDB Feature Store    │
               │   3.75M active semantic edges    │
               └─────────────────┬────────────────┘
                                 │
                     Zero-Copy / Weighted Views
                                 ▼
               ┌──────────────────────────────────┐
               │      Rust Diffusion Engine       │
               │  (Bidirectional PPR / CSR Graph) │
               └─────────────────┬────────────────┘
                                 │
                                 ▼
               Explanatory Subgraph & Recommendations
```

### Edge Weighting (Adamic-Adar / IDF)

To prevent broad categories (*Action*, *Rock*, *Fiction*) from acting as probability sinks and dominating diffusion with generic suggestions, `v_weighted_graph_edges` attenuates edge weights using the inverse log of the destination node's degree:

$$\text{weight}(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{degree}(v))}$$

Rare, specific connections (niche subgenres, auteur directors, or specific soundtrack composers) retain high energy transfer, while mass-vocabulary nodes receive proportional penalty.

---

## Current Graph Scale (DuckDB)

The entire graph operates locally in a single database file (`data/processed/taste_graph.duckdb`):

| Domain / Table | Records | Source / License |
| :--- | :---: | :--- |
| **Movies & Anime** (`movies`) | **34,188** | IMDb Datasets + MyAnimeList Top Anime |
| **TV Series & Mini-Series** (`tv_series`) | **7,647** | IMDb Datasets (`tvSeries`, `tvMiniSeries`) |
| **People & Studios** (`people`) | **37,498** | Directors, writers, and animation studios |
| **Credits & Production** (`movie_credits`) | **106,423** | Direction, writing, and studios (`produced_by`) |
| **Movie/Anime Keywords** (`movie_keywords`) | **239,528** | MovieLens Tag Genome + MyAnimeList |
| **Games** (`games`) | **20,064** | Steam Dataset + Console Classics (Nintendo/PS/Sega) |
| **Developers** (`game_developers`) | **9,250** | Studios and publishers (PC & Consoles) |
| **Game Tags** (`game_tags`) | **133,983** | Community tags and platform tags |
| **Books** (`books`) | **38,203** | Goodreads 100k (CC0 Parquet) |
| **Authors** (`book_authors`) | **29,742** | Authors and co-authors |
| **Literary Themes** (`book_subjects`) | **400,285** | Topics and categories |
| **Musical Artists** (`music_artists`) | **3,257,918** | MusicBrainz + Spotify Mass Ingestion (56M) |
| **Music Albums** (`music_albums`) | **18,034,542** | Full discographies and international releases |
| **Music Tracks** (`music_tracks`) | **40,418,286** | Spotify Tracks with Audio Features (BPM, Valence, Energy) |
| **Music Tags** (`artist_tags`) | **485,064** | Microgenres and folksonomy |
| **Artist Similarities** (`artist_similarities`) | **61,680** | Last.fm Co-listening Graph (HetRec) |
| **Global Keywords** (`keywords`) | **48,263** | Unified thematic vocabulary |
| **Total Nodes (`v_graph_nodes`)** | **61,927,954** | Analytical query in DuckDB |
| **Total Edges (`v_graph_edges`)** | **99,278,228** | Weighted active edges |

---

## Repository Structure

```
youmightlike/
├── crates/
│   └── engine/             # Rust diffusion core and server (PPR, Axum, Rayon)
├── python/
│   ├── src/youmightlike/
│   │   ├── connectors/     # Metadata fetchers (Wikidata, OpenLibrary, MusicBrainz)
│   │   ├── storage/        # SQL schema, DuckDB connection, and bulk ingestion scripts
│   │   └── web/            # Interactive constellation Canvas UI assets
├── data/                   # Ignored in git
│   ├── raw/                # Compressed dumps (IMDb, Steam, Goodreads, MusicBrainz, Last.fm)
│   └── processed/          # taste_graph.duckdb
└── docs/                   # Architecture and dataset documentation
```

---

## Running Locally

### 1. Prerequisites
* Python 3.11+
* Rust & Cargo

### 2. Python Environment Setup
```bash
# Create and activate virtual environment
python3 -m venv .venv
source .venv/bin/activate

# Install package in editable mode
pip install -e python/
```

### 3. Bulk Data Ingestions (Optional if database is already generated)
The DuckDB database is generated directly from columnar source files:
```bash
# Movie ingestion (IMDb TSVs)
python python/src/youmightlike/storage/ingest_imdb.py

# Movie semantics and tags (MovieLens Tag Genome)
python python/src/youmightlike/storage/ingest_movielens.py

# Anime, studios, and themes (MyAnimeList)
python python/src/youmightlike/storage/ingest_anime.py

# Modern games (Steam Parquet)
python python/src/youmightlike/storage/ingest_steam.py

# Console classics (Nintendo, PlayStation, Sega)
python python/src/youmightlike/storage/ingest_console_games.py

# Books ingestion (Goodreads Parquet)
python python/src/youmightlike/storage/ingest_books.py

# Music artists and discographies (MusicBrainz Parquet + Spotify CSV)
python python/src/youmightlike/storage/ingest_music.py

# Individual music tracks with audio features (Spotify)
python python/src/youmightlike/storage/ingest_tracks.py

# Direct artist similarities (Last.fm Graph)
python python/src/youmightlike/storage/ingest_lastfm_similars.py
```

### 4. Start the Interactive Web Interface (Native Canvas UI in Rust)
The HTTP web service and CSR diffusion engine run natively in Rust (`youmightlike-server` via Axum/Tokio), keeping the semantic graph in memory for sub-second responses:
```bash
# Build release binaries (engine + server)
cargo build --release

# Run Rust server directly on port 8000
./target/release/youmightlike-server --port 8000

# Or run via Python module wrapper (launches native Rust binary)
python -m youmightlike.web
```
Open `http://localhost:8000` in your browser:
- **Seed bar:** Add favorite seeds (tracks, games, movies, TV series, or artists).
- **Topological Canvas:** Explore interactive semantic edges rendered with a zero-freeze native HTML5 Canvas 3-ring constellation.
- **Language Switcher [ PT | EN ]:** Toggle between Portuguese and English instantly.
- **Shift + Click:** Inspect and highlight multi-node paths and mutual bridges.

### 5. Querying Graph via Python / CLI
You can inspect the graph from the command line or via DBeaver in `read_only` mode:
```python
import duckdb

conn = duckdb.connect("data/processed/taste_graph.duckdb", read_only=True)
print("Nodes:", conn.execute("SELECT count(*) FROM v_graph_nodes").fetchone()[0])
print("Edges:", conn.execute("SELECT count(*) FROM v_graph_edges").fetchone()[0])
```

---

## License

MIT. External datasets adhere to their respective open licenses (CC0, ODC-By, and open community datasets).
