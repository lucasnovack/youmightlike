#!/usr/bin/env python3
"""
export_parquets_for_hf.py — Exports all processed cultural and transmedia tables
from DuckDB into optimized, zstd-compressed Parquet files ready for Hugging Face Hub.

Includes:
- Complete cultural catalogs: movies, anime, tv series, games, books, creators.
- Transmedia links: adaptations (Wikidata SPARQL), soundtracks, co-play, co-listening.
- Massive music catalog: 3.25M artists, 18M albums, 40.4M tracks with audio features.
- Unified diffusion graph: graph_semantic_edges.
- Auto-generated Dataset Card README.md for Hugging Face with schema & DuckDB instructions.
"""

import argparse
import os
import sys
import time
import duckdb

DATASET_CARD_TEMPLATE = """---
language:
- en
- pt
license: mit
task_categories:
- recommendation
- feature-extraction
- graph-machine-learning
tags:
- recommendation-engine
- transmedia
- culture
- music
- movies
- games
- books
- pagerank
- duckdb
- parquet
size_categories:
- 10M<n<100M
---

# `youmightlike`: Transmedia Cultural Affinity Graph & Catalog

This dataset is the complete feature store and semantic affinity graph behind [`youmightlike`](https://github.com/lucasnovack/youmightlike), connecting **Cinema, TV Series, Music, Literature, and Video Games** into a unified cross-media recommendation engine.

Instead of isolated domain silos, deep cultural relationships—soundtrack compositions, literary adaptations, studio lineages, microgenres, co-listening patterns, and conceptual tropes—are projected onto an IDF-attenuated graph in DuckDB / Parquet.

## 📊 Dataset Summary

| Dimension / Table | Records | Format | Description |
| :--- | :---: | :---: | :--- |
| **`music_tracks.parquet`** | **40,418,287** | Parquet (ZSTD) | Spotify + MusicBrainz catalog with audio features (tempo, energy, valence, danceability) |
| **`music_albums.parquet`** | **18,034,542** | Parquet (ZSTD) | Studio albums, EPs, singles, and OST releases |
| **`music_artists.parquet`** | **3,257,918** | Parquet (ZSTD) | Canonical artists, bands, producers, and composers |
| **`graph_semantic_edges.parquet`** | **1,956,347** | Parquet (ZSTD) | Active bidirectional edges with base weights for PageRank diffusion |
| **`artist_tags.parquet`** | **623,092** | Parquet (ZSTD) | Microgenres, era, and atmosphere tags |
| **`book_subjects.parquet`** | **400,285** | Parquet (ZSTD) | Goodreads genres, subjects, and literary tropes |
| **`movie_keywords.parquet`** | **239,483** | Parquet (ZSTD) | MovieLens Tag Genome & thematic descriptors |
| **`artist_similarities.parquet`** | **170,814** | Parquet (ZSTD) | Last.fm HetRec co-audition similarity scores |
| **`movie_credits.parquet`** | **106,423** | Parquet (ZSTD) | Direction, writing, and studio relationships |
| **`series_credits.parquet`** | **152,489** | Parquet (ZSTD) | Series creators, head writers, and showrunners |
| **`people.parquet`** | **82,702** | Parquet (ZSTD) | Directors, actors, writers, and animators |
| **`game_tags.parquet`** | **73,510** | Parquet (ZSTD) | Steam thematic tags and platform identifiers |
| **`books.parquet`** | **41,940** | Parquet (ZSTD) | Canonical books (Goodreads CC0) |
| **`movies.parquet`** | **34,188** | Parquet (ZSTD) | Feature films and anime (IMDb + MyAnimeList) |
| **`book_authors.parquet`** | **31,675** | Parquet (ZSTD) | Literary authors and co-authors |
| **`game_similarities.parquet`** | **20,854** | Parquet (ZSTD) | Steam user playtime co-play cosine similarity |
| **`games.parquet`** | **20,064** | Parquet (ZSTD) | PC & console titles (Steam + Console Classics) |
| **`game_developers.parquet`** | **8,358** | Parquet (ZSTD) | Game studios and publishers |
| **`tv_series.parquet`** | **7,647** | Parquet (ZSTD) | TV series and mini-series (IMDb) |
| **`movie_soundtrack_links.parquet`** | **103** | Parquet (ZSTD) | Curated direct movie-to-track soundtrack links |

---

## ⚡ Instant Querying with DuckDB (No Download Required)

You can query any table directly over HTTP using DuckDB without downloading the full gigabytes:

```sql
-- Direct query against 40 million tracks:
SELECT title, popularity, tempo, energy, valence
FROM read_parquet('https://huggingface.co/datasets/{repo_id}/resolve/main/music_tracks.parquet')
WHERE popularity > 80
ORDER BY popularity DESC
LIMIT 10;
```

Or query the graph in Python:

```python
import duckdb

conn = duckdb.connect()
df = conn.execute(\"\"\"
    SELECT source_id, target_id, relation_type, base_weight
    FROM read_parquet('https://huggingface.co/datasets/{repo_id}/resolve/main/graph_semantic_edges.parquet')
    WHERE source_id LIKE 'movie:%'
    LIMIT 10;
\"\"\").df()
print(df)
```

---

## 📚 Licensing & Attribution

* Datasets compiled from public/open metadata: IMDb non-commercial, MusicBrainz (CC0 / Open Data), Goodreads (CC0), Steam, Last.fm HetRec, and Wikidata (CC0).
* Source code repository: [https://github.com/lucasnovack/youmightlike](https://github.com/lucasnovack/youmightlike)
"""

def parse_args():
    parser = argparse.ArgumentParser(description="Export DuckDB tables to Parquet for Hugging Face Hub")
    parser.add_argument(
        "--source",
        default="data/processed/taste_graph.duckdb",
        help="Path to full source DuckDB (default: data/processed/taste_graph.duckdb)",
    )
    parser.add_argument(
        "--output-dir",
        default="data/parquet_export",
        help="Output directory for Parquet files (default: data/parquet_export)",
    )
    parser.add_argument(
        "--repo-id",
        default="lucasnovack/youmightlike-dataset",
        help="Hugging Face repo id to interpolate into README (default: lucasnovack/youmightlike-dataset)",
    )
    return parser.parse_args()

def main():
    args = parse_args()

    if not os.path.exists(args.source):
        print(f"Error: Source database '{args.source}' does not exist.")
        sys.exit(1)

    os.makedirs(args.output_dir, exist_ok=True)

    print("=" * 65)
    print(f" Exporting DuckDB to Parquet: {args.source} -> {args.output_dir}/")
    print("=" * 65)

    conn = duckdb.connect(args.source, read_only=True)
    conn.execute("PRAGMA threads=8; PRAGMA memory_limit='8GB';")

    tables = [
        "movies",
        "tv_series",
        "games",
        "books",
        "people",
        "game_developers",
        "book_authors",
        "keywords",
        "genres",
        "movie_credits",
        "movie_genres",
        "movie_keywords",
        "movie_soundtrack_links",
        "series_credits",
        "series_genres",
        "series_keywords",
        "game_dev_links",
        "game_tags",
        "game_similarities",
        "book_author_links",
        "book_subjects",
        "graph_semantic_edges",
        "music_artists",
        "artist_tags",
        "artist_similarities",
        "music_albums",
        "music_tracks",
    ]

    total_start = time.time()
    total_bytes = 0

    for i, table in enumerate(tables, 1):
        parquet_path = os.path.join(args.output_dir, f"{table}.parquet")
        t0 = time.time()
        print(f"[{i:02d}/{len(tables)}] Exporting '{table}'...", end="", flush=True)

        conn.execute(f"COPY {table} TO '{parquet_path}' (FORMAT PARQUET, COMPRESSION ZSTD);")

        elapsed = time.time() - t0
        file_size = os.path.getsize(parquet_path)
        total_bytes += file_size
        size_mb = file_size / (1024 * 1024)
        print(f" Done in {elapsed:.2f}s ({size_mb:.1f} MB)")

    # Write Hugging Face Dataset Card README
    readme_path = os.path.join(args.output_dir, "README.md")
    with open(readme_path, "w", encoding="utf-8") as f:
        f.write(DATASET_CARD_TEMPLATE.replace("{repo_id}", args.repo_id))

    conn.close()

    total_time = time.time() - total_start
    total_gb = total_bytes / (1024 * 1024 * 1024)

    print("=" * 65)
    print(f" SUCCESS! All {len(tables)} tables exported in {total_time:.1f}s")
    print(f" Total Parquet size: {total_gb:.2f} GB (ZSTD compressed)")
    print(f" Dataset Card written: {readme_path}")
    print("=" * 65)

if __name__ == "__main__":
    main()
