"""
Prune homonym `is_artist` bridges (person -> music artist) from graph_semantic_edges.

Context: `is_artist` edges were materialized from `v_graph_edges`, whose fallback branch joins
people and music artists by *name only*. Since almost no music artist has a Wikidata QID,
~99.9% of the bridges came from that fallback, linking screenwriters/directors/actors to
unrelated homonym musicians (e.g. director Tony Scott -> a musician named Tony Scott).

Keep rule (an edge survives if ANY holds):
  1. Identity match: people.wikidata_id = music_artists.wikidata_id
  2. Music role: the person is the target of a `has_composer` edge (credited film/series composer)

Removed rows are archived in `graph_semantic_edges_pruned` (with reason + timestamp), so the
operation is reversible:
    INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
    SELECT source_id, target_id, base_weight, relation_type
    FROM graph_semantic_edges_pruned WHERE reason = 'is_artist_homonym';

Usage (youmightlike-server must be stopped for --apply):
    python -m youmightlike.prune_is_artist_homonyms            # dry-run report
    python -m youmightlike.prune_is_artist_homonyms --apply    # archive + delete
"""

import argparse
import time

import duckdb

MAIN_DB = "data/processed/taste_graph.duckdb"
REASON = "is_artist_homonym"

INVALID_EDGES_SQL = """
    SELECT e.source_id, e.target_id, e.base_weight, e.relation_type
    FROM graph_semantic_edges e
    JOIN people p ON p.id = CAST(split_part(e.source_id, ':', 2) AS BIGINT)
    JOIN music_artists a ON a.id = CAST(split_part(e.target_id, ':', 2) AS BIGINT)
    WHERE e.relation_type = 'is_artist'
      AND (p.wikidata_id IS NULL OR a.wikidata_id IS NULL OR p.wikidata_id != a.wikidata_id)
      AND e.source_id NOT IN (
          SELECT DISTINCT target_id FROM graph_semantic_edges WHERE relation_type = 'has_composer' AND target_id IS NOT NULL
      )
"""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--db", default=MAIN_DB)
    parser.add_argument("--apply", action="store_true", help="Archive and delete invalid edges")
    args = parser.parse_args()

    try:
        con = duckdb.connect(args.db, read_only=not args.apply)
    except Exception as e:
        print(f"Error opening {args.db}: {e}")
        print("Make sure youmightlike-server is stopped before running with --apply.")
        return

    total = con.execute("SELECT COUNT(*) FROM graph_semantic_edges WHERE relation_type = 'is_artist'").fetchone()[0]
    invalid = con.execute(f"SELECT COUNT(*) FROM ({INVALID_EDGES_SQL})").fetchone()[0]
    print(f"is_artist edges: {total:,} total | {invalid:,} invalid (homonyms) | {total - invalid:,} kept")

    print("\nSample of edges to remove (person -> artist):")
    sample = con.execute(f"""
        SELECT p.name AS person, a.name AS artist
        FROM ({INVALID_EDGES_SQL}) x
        JOIN people p ON p.id = CAST(split_part(x.source_id, ':', 2) AS BIGINT)
        JOIN music_artists a ON a.id = CAST(split_part(x.target_id, ':', 2) AS BIGINT)
        USING SAMPLE 10 ROWS
    """).fetchall()
    for person, artist in sample:
        print(f"  {person}  ->  {artist}")

    print("\nKept edges (person -> artist):")
    kept = con.execute(f"""
        SELECT p.name, a.name
        FROM graph_semantic_edges e
        JOIN people p ON p.id = CAST(split_part(e.source_id, ':', 2) AS BIGINT)
        JOIN music_artists a ON a.id = CAST(split_part(e.target_id, ':', 2) AS BIGINT)
        WHERE e.relation_type = 'is_artist'
          AND (e.source_id, e.target_id) NOT IN (SELECT source_id, target_id FROM ({INVALID_EDGES_SQL}))
        LIMIT 40
    """).fetchall()
    for person, artist in kept:
        print(f"  {person}  ->  {artist}")

    if not args.apply:
        print("\nDry-run only. Re-run with --apply to archive and delete.")
        con.close()
        return

    t0 = time.time()
    con.execute("BEGIN TRANSACTION")
    con.execute("""
        CREATE TABLE IF NOT EXISTS graph_semantic_edges_pruned (
            source_id VARCHAR, target_id VARCHAR, base_weight DOUBLE, relation_type VARCHAR,
            reason VARCHAR, pruned_at TIMESTAMP
        )
    """)
    con.execute(f"""
        INSERT INTO graph_semantic_edges_pruned
        SELECT source_id, target_id, base_weight, relation_type, '{REASON}', now()
        FROM ({INVALID_EDGES_SQL})
    """)
    con.execute(f"""
        DELETE FROM graph_semantic_edges
        WHERE relation_type = 'is_artist'
          AND (source_id, target_id) IN (
              SELECT source_id, target_id FROM graph_semantic_edges_pruned WHERE reason = '{REASON}'
          )
    """)
    con.execute("COMMIT")

    remaining = con.execute("SELECT COUNT(*) FROM graph_semantic_edges WHERE relation_type = 'is_artist'").fetchone()[0]
    print(f"\nApplied in {time.time() - t0:.2f}s. is_artist edges remaining: {remaining:,}")
    con.close()


if __name__ == "__main__":
    main()
