#!/usr/bin/env python3
"""
export_lite_db.py — Generates a lightweight, high-performance DuckDB database (taste_graph_lite.duckdb)
tailored for low-memory production deployments (GCP Always Free, Cloud Run, low-end VPS).

Preserves:
- 100% of Movies, Anime, TV Series, Games, Books, Authors, People, Tropes and Tags.
- 100% of Active Semantic Edges (the transmedia PPR diffusion graph).
- Active artists with semantic connections (122k vs 3.2M unlinked stubs).
- Top 3 popular tracks per active artist and soundtrack connections (250k vs 40M raw tracks).
"""

import argparse
import os
import sys
import time
import duckdb

def parse_args():
    parser = argparse.ArgumentParser(description="Export lightweight DuckDB for youmightlike demo")
    parser.add_argument(
        "--source",
        default="data/processed/taste_graph.duckdb",
        help="Path to full source DuckDB (default: data/processed/taste_graph.duckdb)",
    )
    parser.add_argument(
        "--output",
        default="data/processed/taste_graph_lite.duckdb",
        help="Path to output lite DuckDB (default: data/processed/taste_graph_lite.duckdb)",
    )
    parser.add_argument(
        "--top-tracks-per-artist",
        type=int,
        default=3,
        help="Number of top popular tracks to retain per active artist (default: 3)",
    )
    return parser.parse_args()

def main():
    args = parse_args()

    if not os.path.exists(args.source):
        print(f"Error: Source database '{args.source}' does not exist.")
        sys.exit(1)

    output_dir = os.path.dirname(args.output)
    if output_dir and not os.path.exists(output_dir):
        os.makedirs(output_dir, exist_ok=True)

    if os.path.exists(args.output):
        print(f"Removing existing target database at {args.output}...")
        os.remove(args.output)

    print(f"============================================================")
    print(f" Exporting Lite DuckDB: {args.source} -> {args.output}")
    print(f"============================================================")
    t_start = time.time()

    # Open target DB and attach source DB as read-only
    dest_conn = duckdb.connect(args.output)
    dest_conn.execute("PRAGMA threads=8; PRAGMA memory_limit='8GB';")
    dest_conn.execute(f"ATTACH '{args.source}' AS src (READ_ONLY);")

    # 1. Direct copy of non-music tables and semantic graph
    full_copy_tables = [
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
    ]

    for table in full_copy_tables:
        t0 = time.time()
        print(f"  [1/4] Copying table '{table}'...", end="", flush=True)
        dest_conn.execute(f"CREATE TABLE {table} AS SELECT * FROM src.{table};")
        cnt = dest_conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
        print(f" Done ({cnt:,} rows in {time.time() - t0:.2f}s)")

    # Optional table
    has_pruned = dest_conn.execute(
        "SELECT count(*) FROM duckdb_tables() WHERE database_name = 'src' AND table_name = 'graph_semantic_edges_pruned'"
    ).fetchone()[0]
    if has_pruned:
        dest_conn.execute("CREATE TABLE graph_semantic_edges_pruned AS SELECT * FROM src.graph_semantic_edges_pruned;")

    # 2. Extract active artists
    print(f"  [2/4] Identifying active musical artists from semantic graph...")
    t0 = time.time()
    dest_conn.execute("""
        CREATE TEMP TABLE active_artist_ids AS
        SELECT DISTINCT CAST(REPLACE(source_id, 'artist:', '') AS BIGINT) as artist_id
        FROM graph_semantic_edges
        WHERE source_id LIKE 'artist:%'
        UNION
        SELECT DISTINCT CAST(REPLACE(target_id, 'artist:', '') AS BIGINT) as artist_id
        FROM graph_semantic_edges
        WHERE target_id LIKE 'artist:%'
        UNION
        SELECT DISTINCT artist_id FROM movie_soundtrack_links WHERE artist_id IS NOT NULL;
    """)
    active_artists_count = dest_conn.execute("SELECT count(*) FROM active_artist_ids;").fetchone()[0]
    print(f"        Found {active_artists_count:,} active artists in {time.time() - t0:.2f}s")

    # Copy filtered music_artists
    t0 = time.time()
    print("  [2/4] Copying filtered 'music_artists'...", end="", flush=True)
    dest_conn.execute("""
        CREATE TABLE music_artists AS
        SELECT a.*
        FROM src.music_artists a
        JOIN active_artist_ids act ON act.artist_id = a.id;
    """)
    art_cnt = dest_conn.execute("SELECT count(*) FROM music_artists;").fetchone()[0]
    print(f" Done ({art_cnt:,} rows in {time.time() - t0:.2f}s)")

    # Copy filtered artist_tags
    t0 = time.time()
    print("  [2/4] Copying filtered 'artist_tags'...", end="", flush=True)
    dest_conn.execute("""
        CREATE TABLE artist_tags AS
        SELECT t.*
        FROM src.artist_tags t
        JOIN active_artist_ids act ON act.artist_id = t.artist_id;
    """)
    tag_cnt = dest_conn.execute("SELECT count(*) FROM artist_tags;").fetchone()[0]
    print(f" Done ({tag_cnt:,} rows in {time.time() - t0:.2f}s)")

    # Copy filtered artist_similarities
    t0 = time.time()
    print("  [2/4] Copying filtered 'artist_similarities'...", end="", flush=True)
    dest_conn.execute("""
        CREATE TABLE artist_similarities AS
        SELECT s.*
        FROM src.artist_similarities s
        JOIN active_artist_ids a1 ON a1.artist_id = s.source_artist_id
        JOIN active_artist_ids a2 ON a2.artist_id = s.target_artist_id;
    """)
    sim_cnt = dest_conn.execute("SELECT count(*) FROM artist_similarities;").fetchone()[0]
    print(f" Done ({sim_cnt:,} rows in {time.time() - t0:.2f}s)")

    # 3. Extract top tracks and soundtrack tracks
    print(f"  [3/4] Filtering top tracks (top {args.top_tracks_per_artist} per active artist + soundtracks)...", end="", flush=True)
    t0 = time.time()
    dest_conn.execute(f"""
        CREATE TEMP TABLE active_track_ids AS
        SELECT DISTINCT CAST(REPLACE(source_id, 'track:', '') AS BIGINT) as track_id
        FROM graph_semantic_edges
        WHERE source_id LIKE 'track:%'
        UNION
        SELECT DISTINCT CAST(REPLACE(target_id, 'track:', '') AS BIGINT) as track_id
        FROM graph_semantic_edges
        WHERE target_id LIKE 'track:%'
        UNION
        SELECT DISTINCT track_id FROM movie_soundtrack_links WHERE track_id IS NOT NULL;
    """)

    dest_conn.execute(f"""
        CREATE TABLE music_tracks AS
        WITH ranked_tracks AS (
            SELECT 
                t.*,
                ROW_NUMBER() OVER (
                    PARTITION BY t.artist_id 
                    ORDER BY t.popularity DESC NULLS LAST, t.id ASC
                ) as rn
            FROM src.music_tracks t
            JOIN active_artist_ids act ON act.artist_id = t.artist_id
        )
        SELECT 
            id, spotify_id, artist_id, album_id, title, duration_ms, 
            popularity, danceability, energy, tempo, valence, acousticness, instrumentalness
        FROM ranked_tracks
        WHERE rn <= {args.top_tracks_per_artist}
        UNION
        SELECT 
            t.id, t.spotify_id, t.artist_id, t.album_id, t.title, t.duration_ms, 
            t.popularity, t.danceability, t.energy, t.tempo, t.valence, t.acousticness, t.instrumentalness
        FROM src.music_tracks t
        JOIN active_track_ids act_t ON act_t.track_id = t.id;
    """)
    trk_cnt = dest_conn.execute("SELECT count(*) FROM music_tracks;").fetchone()[0]
    print(f" Done ({trk_cnt:,} rows in {time.time() - t0:.2f}s)")

    # Copy music_albums associated with selected tracks
    t0 = time.time()
    print("  [3/4] Copying filtered 'music_albums'...", end="", flush=True)
    dest_conn.execute("""
        CREATE TABLE music_albums AS
        SELECT alb.*
        FROM src.music_albums alb
        WHERE alb.id IN (SELECT DISTINCT album_id FROM music_tracks WHERE album_id IS NOT NULL);
    """)
    alb_cnt = dest_conn.execute("SELECT count(*) FROM music_albums;").fetchone()[0]
    print(f" Done ({alb_cnt:,} rows in {time.time() - t0:.2f}s)")

    # 4. Create essential performance indices & checkpoint
    print("  [4/4] Creating database indexes & running checkpoint...", end="", flush=True)
    t0 = time.time()
    dest_conn.execute("CREATE INDEX IF NOT EXISTS idx_artists_name ON music_artists(name);")
    dest_conn.execute("CREATE INDEX IF NOT EXISTS idx_tracks_artist ON music_tracks(artist_id);")
    dest_conn.execute("CHECKPOINT;")
    print(f" Done in {time.time() - t0:.2f}s")

    dest_conn.close()

    size_mb = os.path.getsize(args.output) / (1024 * 1024)
    size_orig_gb = os.path.getsize(args.source) / (1024 * 1024 * 1024)
    total_time = time.time() - t_start

    print(f"============================================================")
    print(f" SUCCESS! Lite database created in {total_time:.1f}s")
    print(f" Original: {size_orig_gb:.2f} GB -> Lite: {size_mb:.1f} MB ({(1.0 - (size_mb / (size_orig_gb * 1024))) * 100:.1f}% reduction)")
    print(f" Target path: {args.output}")
    print(f"============================================================")

if __name__ == "__main__":
    main()
