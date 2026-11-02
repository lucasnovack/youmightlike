from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

SPOTIFY_56M_PARQUET = Path("data/raw/music/spotify_56m_tracks.parquet")


def run_56m_music_ingestion() -> None:
    """Ingere a base massiva de 56 milhões de faixas/créditos do Spotify no DuckDB."""
    if not SPOTIFY_56M_PARQUET.exists():
        raise FileNotFoundError(f"Arquivo massivo do Spotify não encontrado: {SPOTIFY_56M_PARQUET}")

    db = init_db()
    console.print("\n[bold cyan]🎵 Iniciando Ingestão da Base Massiva de 56M de Músicas (Spotify)[/bold cyan]")
    console.print(f"📦 Arquivo fonte: {SPOTIFY_56M_PARQUET} (4.1 GB Parquet)\n")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='16GB';")
        conn.execute("PRAGMA preserve_insertion_order=false;")

        # ---------------------------------------------------------------------
        # 1. IDENTIFICAR E INSERIR ARTISTAS INÉDITOS
        # ---------------------------------------------------------------------
        console.print("[bold yellow]🎤 1/3 Identificando e cadastrando artistas inéditos do Spotify 56M...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_new_artists AS
            WITH raw_artists AS (
                SELECT DISTINCT TRIM(artist_name) AS name
                FROM '{SPOTIFY_56M_PARQUET}'
                WHERE artist_name IS NOT NULL 
                  AND length(TRIM(artist_name)) > 1
                  AND artist_name NOT LIKE '[%'
                  AND artist_name NOT IN ('Various Artists', 'Unknown Artist', 'Traditional')
            ),
            dedup_artists AS (
                SELECT name, ROW_NUMBER() OVER (PARTITION BY LOWER(name) ORDER BY length(name) DESC) AS rn
                FROM raw_artists
            )
            SELECT d.name
            FROM dedup_artists d
            LEFT JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(d.name)
            WHERE d.rn = 1 AND ma.id IS NULL;
            """
        )

        new_artists_count = conn.execute("SELECT COUNT(*) FROM tmp_new_artists").fetchone()[0]
        console.print(f"  -> {new_artists_count:,} novos artistas identificados em {time.time() - t0:.2f}s!")

        if new_artists_count > 0:
            t_ins = time.time()
            conn.execute(
                """
                INSERT INTO music_artists (name)
                SELECT name FROM tmp_new_artists;
                """
            )
            total_artists = conn.execute("SELECT COUNT(*) FROM music_artists").fetchone()[0]
            console.print(f"  -> {new_artists_count:,} artistas inseridos em {time.time() - t_ins:.2f}s. Total atual: {total_artists:,}")

        conn.execute("DROP TABLE IF EXISTS tmp_new_artists;")

        # ---------------------------------------------------------------------
        # 2. IDENTIFICAR E INSERIR ÁLBUNS INÉDITOS
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]💿 2/3 Cadastrando novos álbuns da base 56M...[/bold yellow]")
        t0 = time.time()

        # Cria mapeamento canônico de artistas para evitar produto cartesiano
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_canon_artists_albums AS
            SELECT id, norm_name
            FROM (
                SELECT id, LOWER(TRIM(name)) AS norm_name,
                       ROW_NUMBER() OVER (PARTITION BY LOWER(TRIM(name)) ORDER BY listeners DESC, id ASC) AS rn
                FROM music_artists
            )
            WHERE rn = 1;
            """
        )

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_new_albums AS
            WITH raw_albums AS (
                SELECT DISTINCT
                    TRIM(p.artist_name) AS artist_name,
                    TRIM(p.album_name) AS title,
                    YEAR(p.album_release_date) AS release_year
                FROM '{SPOTIFY_56M_PARQUET}' p
                WHERE p.album_name IS NOT NULL 
                  AND length(TRIM(p.album_name)) > 0
                  AND p.album_name NOT IN ('null', 'None', '[unknown]')
            ),
            resolved_albums AS (
                SELECT 
                    ca.id AS artist_id,
                    r.title,
                    r.release_year,
                    ROW_NUMBER() OVER (PARTITION BY ca.id, LOWER(r.title) ORDER BY r.release_year NULLS LAST) AS rn
                FROM raw_albums r
                JOIN tmp_canon_artists_albums ca ON ca.norm_name = LOWER(r.artist_name)
            )
            SELECT r.artist_id, r.title, r.release_year
            FROM resolved_albums r
            LEFT JOIN music_albums alb ON alb.artist_id = r.artist_id AND LOWER(TRIM(alb.title)) = LOWER(r.title)
            WHERE r.rn = 1 AND alb.id IS NULL;
            """
        )

        new_albums_count = conn.execute("SELECT COUNT(*) FROM tmp_new_albums").fetchone()[0]
        console.print(f"  -> {new_albums_count:,} novos álbuns identificados em {time.time() - t0:.2f}s!")

        if new_albums_count > 0:
            t_ins = time.time()
            conn.execute(
                """
                INSERT INTO music_albums (artist_id, title, release_year)
                SELECT artist_id, title, release_year FROM tmp_new_albums;
                """
            )
            total_albums = conn.execute("SELECT COUNT(*) FROM music_albums").fetchone()[0]
            console.print(f"  -> Álbuns inseridos em {time.time() - t_ins:.2f}s. Total atual: {total_albums:,}")

        conn.execute("DROP TABLE IF EXISTS tmp_canon_artists_albums;")
        conn.execute("DROP TABLE IF EXISTS tmp_new_albums;")

        # ---------------------------------------------------------------------
        # 3. MAPEAMENTO E INSERÇÃO DAS FAIXAS COM AUDIO FEATURES
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]🎼 3/3 Mapeando e inserindo dezenas de milhões de músicas e audio features...[/bold magenta]")
        t0 = time.time()

        console.print("  -> Construindo índices temporários de artistas e álbuns canônicos...")
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_canonical_artists AS
            SELECT id, norm_name
            FROM (
                SELECT id, LOWER(TRIM(name)) AS norm_name,
                       ROW_NUMBER() OVER (PARTITION BY LOWER(TRIM(name)) ORDER BY listeners DESC, id ASC) AS rn
                FROM music_artists
            )
            WHERE rn = 1;

            CREATE OR REPLACE TEMP TABLE tmp_canonical_albums AS
            SELECT id, artist_id, norm_title
            FROM (
                SELECT id, artist_id, LOWER(TRIM(title)) AS norm_title,
                       ROW_NUMBER() OVER (PARTITION BY artist_id, LOWER(TRIM(title)) ORDER BY id ASC) AS rn
                FROM music_albums
            )
            WHERE rn = 1;

            CREATE OR REPLACE TEMP TABLE tmp_existing_tracks AS
            SELECT spotify_id FROM music_tracks WHERE spotify_id IS NOT NULL;
            """
        )

        console.print("  -> Resolvendo faixas do dataset Parquet com garantia de unicidade estrita...")
        t_resolve = time.time()
        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_distinct_tracks AS
            WITH ranked_tracks AS (
                SELECT 
                    p.track_id AS spotify_id,
                    TRIM(p.artist_name) AS artist_name,
                    TRIM(p.album_name) AS album_name,
                    TRIM(p.track_name) AS title,
                    CAST(p.duration_ms AS BIGINT) AS duration_ms,
                    CAST(p.track_popularity AS INTEGER) AS popularity,
                    p.danceability,
                    p.energy,
                    p.tempo,
                    p.valence,
                    p.acousticness,
                    p.instrumentalness,
                    ROW_NUMBER() OVER (PARTITION BY p.track_id ORDER BY p.track_popularity DESC, p.artist_name) AS rn
                FROM '{SPOTIFY_56M_PARQUET}' p
                WHERE p.track_id IS NOT NULL 
                  AND p.track_name IS NOT NULL 
                  AND length(TRIM(p.track_name)) > 0
            )
            SELECT DISTINCT ON (r.spotify_id)
                r.spotify_id,
                ca.id AS artist_id,
                calb.id AS album_id,
                r.title,
                r.duration_ms,
                r.popularity,
                r.danceability,
                r.energy,
                r.tempo,
                r.valence,
                r.acousticness,
                r.instrumentalness
            FROM ranked_tracks r
            LEFT JOIN tmp_existing_tracks ext ON ext.spotify_id = r.spotify_id
            JOIN tmp_canonical_artists ca ON ca.norm_name = LOWER(TRIM(r.artist_name))
            LEFT JOIN tmp_canonical_albums calb ON calb.artist_id = ca.id AND calb.norm_title = LOWER(TRIM(r.album_name))
            WHERE r.rn = 1 AND ext.spotify_id IS NULL;
            """
        )

        new_tracks_count = conn.execute("SELECT COUNT(*) FROM tmp_distinct_tracks").fetchone()[0]
        console.print(f"  -> {new_tracks_count:,} novas faixas qualificadas mapeadas em {time.time() - t_resolve:.2f}s!")

        if new_tracks_count > 0:
            console.print("  -> Gravando novas faixas na tabela canônica 'music_tracks'...")
            t_ins = time.time()
            conn.execute(
                """
                INSERT INTO music_tracks (
                    spotify_id, artist_id, album_id, title, duration_ms, popularity,
                    danceability, energy, tempo, valence, acousticness, instrumentalness
                )
                SELECT 
                    spotify_id, artist_id, album_id, title, duration_ms, popularity,
                    danceability, energy, tempo, valence, acousticness, instrumentalness
                FROM tmp_distinct_tracks;
                """
            )
            total_tracks = conn.execute("SELECT COUNT(*) FROM music_tracks").fetchone()[0]
            console.print(f"  -> Faixas inseridas com sucesso em {time.time() - t_ins:.2f}s! Total de faixas: {total_tracks:,}")

        conn.execute("DROP TABLE IF EXISTS tmp_canonical_artists;")
        conn.execute("DROP TABLE IF EXISTS tmp_canonical_albums;")
        conn.execute("DROP TABLE IF EXISTS tmp_existing_tracks;")
        conn.execute("DROP TABLE IF EXISTS tmp_distinct_tracks;")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]🎉 Ingestão da base 56M concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

        _print_summary(conn)


def _print_summary(conn) -> None:
    table = Table(title="Censo Geral Consolidado do Grafo Transmidiático (DuckDB)", show_header=True, header_style="bold cyan")
    table.add_column("Domínio / Tabela", style="bold")
    table.add_column("Total de Registros", justify="right", style="green")

    tables = [
        ("Filmes & Animes (movies)", "SELECT count(*) FROM movies"),
        ("Pessoas & Estúdios (people)", "SELECT count(*) FROM people"),
        ("Créditos & Produção (movie_credits)", "SELECT count(*) FROM movie_credits"),
        ("Keywords de Filmes/Animes (movie_keywords)", "SELECT count(*) FROM movie_keywords"),
        ("Jogos (games)", "SELECT count(*) FROM games"),
        ("Desenvolvedores (game_developers)", "SELECT count(*) FROM game_developers"),
        ("Links de Desenvolvimento (game_dev_links)", "SELECT count(*) FROM game_dev_links"),
        ("Tags de Jogos (game_tags)", "SELECT count(*) FROM game_tags"),
        ("Livros (books)", "SELECT count(*) FROM books"),
        ("Temas de Livros (book_subjects)", "SELECT count(*) FROM book_subjects"),
        ("Artistas Musicais (music_artists)", "SELECT count(*) FROM music_artists"),
        ("Álbuns Musicais (music_albums)", "SELECT count(*) FROM music_albums"),
        ("Faixas / Músicas (music_tracks)", "SELECT count(*) FROM music_tracks"),
        ("Tags de Música (artist_tags)", "SELECT count(*) FROM artist_tags"),
        ("Similaridades entre Artistas (artist_similarities)", "SELECT count(*) FROM artist_similarities"),
        ("Keywords Globais (keywords)", "SELECT count(*) FROM keywords"),
        ("Nós no Grafo (v_graph_nodes)", "SELECT count(*) FROM v_graph_nodes"),
        ("Arestas Ativas (v_graph_edges)", "SELECT count(*) FROM v_graph_edges"),
    ]

    for name, query in tables:
        count = conn.execute(query).fetchone()[0]
        table.add_row(name, f"{count:,}")

    console.print(table)


if __name__ == "__main__":
    run_56m_music_ingestion()
