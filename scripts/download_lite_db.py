#!/usr/bin/env python3
"""
download_lite_db.py — Initializes the lightweight taste_graph_lite.duckdb database
directly from the public Hugging Face dataset (lnvck/youmightlike-dataset).

Can assemble the database in seconds using DuckDB's in-process Parquet streaming
or download from a direct mirror.
"""

import os
import sys
import time
import argparse
import duckdb

HF_BASE_URL = "https://huggingface.co/datasets/lnvck/youmightlike-dataset/resolve/main"

LITE_TABLES = [
    "movies",
    "tv_series",
    "people",
    "movie_credits",
    "movie_genres",
    "movie_keywords",
    "series_credits",
    "series_genres",
    "series_keywords",
    "games",
    "game_developers",
    "game_dev_links",
    "game_tags",
    "game_similarities",
    "books",
    "book_authors",
    "book_author_links",
    "book_subjects",
    "genres",
    "keywords",
    "artist_tags",
    "artist_similarities",
    "movie_soundtrack_links",
    "graph_semantic_edges",
]

def parse_args():
    parser = argparse.ArgumentParser(description="Download and build taste_graph_lite.duckdb from Hugging Face")
    parser.add_argument(
        "--output",
        default="data/processed/taste_graph_lite.duckdb",
        help="Destination path for the DuckDB file (default: data/processed/taste_graph_lite.duckdb)"
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite existing database"
    )
    return parser.parse_args()

def main():
    args = parse_args()
    out_path = args.output

    if os.path.exists(out_path) and not args.force:
        print(f"[OK] Database already exists at {out_path} ({os.path.getsize(out_path) / (1024*1024):.1f} MB).")
        print("     Use --force to rebuild.")
        return

    out_dir = os.path.dirname(out_path)
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)

    print(f"Connecting to DuckDB and streaming tables from Hugging Face:")
    print(f"Repository: lnvck/youmightlike-dataset")
    print(f"Destination: {out_path}\n")

    t0 = time.time()
    conn = duckdb.connect(out_path)
    conn.execute("INSTALL httpfs; LOAD httpfs;")
    conn.execute("SET preserve_insertion_order=false;")

    for i, table in enumerate(LITE_TABLES, start=1):
        url = f"{HF_BASE_URL}/{table}.parquet"
        print(f"[{i}/{len(LITE_TABLES)}] Ingesting {table}...")
        try:
            conn.execute(f"CREATE TABLE {table} AS SELECT * FROM read_parquet('{url}');")
        except Exception as e:
            print(f"      Warning: could not ingest {table} ({e})")

    # Ingest active subset of music catalog (artists + top tracks)
    print(f"[{len(LITE_TABLES)+1}/{len(LITE_TABLES)+2}] Ingesting active music artists...")
    try:
        conn.execute(f"CREATE TABLE music_artists AS SELECT * FROM read_parquet('{HF_BASE_URL}/music_artists.parquet');")
    except Exception as e:
        print(f"      Warning: could not ingest music_artists ({e})")

    print(f"[{len(LITE_TABLES)+2}/{len(LITE_TABLES)+2}] Creating music tracks index...")
    try:
        # Stream top tracks for artists present in active graph
        conn.execute(f"""
            CREATE TABLE music_tracks AS 
            SELECT * FROM read_parquet('{HF_BASE_URL}/music_tracks.parquet')
            WHERE popularity >= 30;
        """)
    except Exception as e:
        print(f"      Warning: could not ingest music_tracks ({e})")

    conn.close()
    elapsed = time.time() - t0
    size_mb = os.path.getsize(out_path) / (1024 * 1024) if os.path.exists(out_path) else 0.0
    print(f"\n[DONE] Successfully assembled {out_path} ({size_mb:.1f} MB) in {elapsed:.1f}s.")

if __name__ == "__main__":
    main()
