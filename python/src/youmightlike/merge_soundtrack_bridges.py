"""
Merge staged game soundtrack bridges into taste_graph.duckdb.
Can be executed when youmightlike-server is not running.
"""

import time
import duckdb

MAIN_DB = "data/processed/taste_graph.duckdb"
STAGE_DB = "data/processed/soundtrack_bridges.duckdb"

def main():
    print("Connecting to taste_graph.duckdb (read-write)...")
    try:
        con = duckdb.connect(MAIN_DB)
    except Exception as e:
        print(f"Error opening taste_graph.duckdb: {e}")
        print("Make sure youmightlike-server is stopped before running this script.")
        return

    t0 = time.time()
    print("Attaching staging database...")
    con.execute(f"ATTACH '{STAGE_DB}' AS stage_db (READ_ONLY)")

    before_count = con.execute("SELECT COUNT(*) FROM graph_semantic_edges").fetchone()[0]
    print(f"Total edges before merge: {before_count}")

    print("Merging soundtrack edges into graph_semantic_edges...")
    con.execute("""
        INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
        SELECT s.source_id, s.target_id, s.relation_type, s.base_weight
        FROM stage_db.game_soundtrack_bridges s
        WHERE NOT EXISTS (
            SELECT 1 FROM graph_semantic_edges e
            WHERE e.source_id = s.source_id
              AND e.target_id = s.target_id
              AND e.relation_type = s.relation_type
        )
    """)

    after_count = con.execute("SELECT COUNT(*) FROM graph_semantic_edges").fetchone()[0]
    soundtrack_edges = con.execute("SELECT COUNT(*) FROM graph_semantic_edges WHERE relation_type IN ('soundtrack_composer', 'soundtrack_for')").fetchone()[0]
    print(f"Merged successfully in {time.time() - t0:.2f}s!")
    print(f"Total edges after merge: {after_count} (+{after_count - before_count} new edges)")
    print(f"Total soundtrack bridges in graph: {soundtrack_edges}")

    con.close()

if __name__ == "__main__":
    main()
