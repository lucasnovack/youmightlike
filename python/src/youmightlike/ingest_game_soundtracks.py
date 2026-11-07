"""
Ingest canonical game soundtrack bridges between games and music artists into DuckDB.
Matches game titles with music albums tagged as soundtracks/OSTs and extracts verified composers/artists.
"""

import time

import duckdb

DB_PATH = "data/processed/taste_graph.duckdb"

def main():
    print("Connecting to DuckDB database...")
    con = duckdb.connect(DB_PATH)

    start_time = time.time()
    print("Mining high-precision game soundtrack connections...")

    # Step 1: Create a staging table with high-precision matches
    con.execute("""
        CREATE TEMP TABLE stage_game_soundtracks AS
        WITH clean_games AS (
            SELECT id AS game_id, title AS game_title,
                   lower(regexp_replace(regexp_replace(title, '\\b(the|a|an)\\b', '', 'g'), '[^a-zA-Z0-9]', '', 'g')) as norm_title
            FROM games
            WHERE length(regexp_replace(title, '[^a-zA-Z0-9]', '', 'g')) >= 4
        ),
        clean_albums AS (
            SELECT a.id AS album_id, a.title AS album_title, a.artist_id, ar.name AS artist_name,
                   lower(regexp_replace(regexp_replace(a.title, '\\b(the|a|an)\\b', '', 'g'), '[^a-zA-Z0-9]', '', 'g')) as norm_album
            FROM music_albums a
            JOIN music_artists ar ON a.artist_id = ar.id
            WHERE (
                a.title ILIKE '%original game soundtrack%' 
                OR a.title ILIKE '%original video game soundtrack%'
                OR a.title ILIKE '%video game soundtrack%'
                OR a.title ILIKE '%original game score%'
                OR a.title ILIKE '%(original soundtrack)%'
                OR a.title ILIKE '%[original soundtrack]%'
                OR a.title ILIKE '%original soundtrack%'
                OR a.title ILIKE '%(ost)%'
                OR a.title ILIKE '%[ost]%'
            )
            AND ar.name NOT IN ('Various Artists', 'Original Soundtrack', 'Soundtrack', 'Various', 'Various Composers')
            AND ar.name NOT ILIKE '%tribute%'
            AND a.title NOT ILIKE '%tribute%'
            AND a.title NOT ILIKE '%cover%'
        )
        SELECT DISTINCT 
            g.game_id,
            g.game_title,
            ca.artist_id,
            ca.artist_name,
            ca.album_title
        FROM clean_games g
        JOIN clean_albums ca ON (
            ca.norm_album = g.norm_title || 'originalgamesoundtrack'
            OR ca.norm_album = g.norm_title || 'originalsoundtrack'
            OR ca.norm_album = g.norm_title || 'videogamesoundtrack'
            OR ca.norm_album = g.norm_title || 'originalvideogamesoundtrack'
            OR ca.norm_album = g.norm_title || 'originalgamescore'
            OR ca.norm_album = g.norm_title || 'ost'
            OR (length(g.norm_title) >= 6 AND ca.norm_album LIKE g.norm_title || '%original%soundtrack%')
            OR (length(g.norm_title) >= 6 AND ca.norm_album LIKE g.norm_title || '%original%score%')
            OR (length(g.norm_title) >= 6 AND ca.norm_album LIKE g.norm_title || '%ost%')
        )
    """)

    match_count = con.execute("SELECT COUNT(*) FROM stage_game_soundtracks").fetchone()[0]
    print(f"Discovered {match_count} high-confidence game soundtrack links in {time.time() - start_time:.2f}s")

    # Step 2: Insert bidirectional edges into graph_semantic_edges
    # Edge A: game -> artist (soundtrack_composer) with base_weight = 3.0
    # Edge B: artist -> game (soundtrack_for) with base_weight = 3.0
    print("Inserting bidirectional semantic edges into graph_semantic_edges...")

    insert_start = time.time()

    con.execute("""
        INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
        SELECT 
            'game:' || game_id AS source_id,
            'artist:' || artist_id AS target_id,
            'soundtrack_composer' AS relation_type,
            3.0 AS base_weight
        FROM stage_game_soundtracks
        WHERE NOT EXISTS (
            SELECT 1 FROM graph_semantic_edges e 
            WHERE e.source_id = 'game:' || stage_game_soundtracks.game_id 
              AND e.target_id = 'artist:' || stage_game_soundtracks.artist_id 
              AND e.relation_type = 'soundtrack_composer'
        );
    """)

    con.execute("""
        INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
        SELECT 
            'artist:' || artist_id AS source_id,
            'game:' || game_id AS target_id,
            'soundtrack_for' AS relation_type,
            3.0 AS base_weight
        FROM stage_game_soundtracks
        WHERE NOT EXISTS (
            SELECT 1 FROM graph_semantic_edges e 
            WHERE e.source_id = 'artist:' || stage_game_soundtracks.artist_id 
              AND e.target_id = 'game:' || stage_game_soundtracks.game_id 
              AND e.relation_type = 'soundtrack_for'
        );
    """)

    print(f"Edges successfully inserted in {time.time() - insert_start:.2f}s")

    total_edges = con.execute("SELECT COUNT(*) FROM graph_semantic_edges WHERE relation_type IN ('soundtrack_composer', 'soundtrack_for')").fetchone()[0]
    print(f"Total active soundtrack bridges in graph: {total_edges}")

    con.close()
    print("Done!")

if __name__ == "__main__":
    main()
