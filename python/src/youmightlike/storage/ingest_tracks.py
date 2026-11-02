from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

SPOTIFY_PARQUET = Path("data/raw/music/spotify_tracks_huge.parquet")
SPOTIFY_CSV = Path("data/raw/music/spotify_tracks.csv")
MB_PARQUET = Path("data/raw/music/musicbrainz_artists.parquet")


def run_tracks_bulk_ingestion() -> None:
    """Ingere faixas musicais individuais com métricas de áudio conectando a artistas e álbuns no DuckDB."""
    if not SPOTIFY_PARQUET.exists() and not SPOTIFY_CSV.exists():
        raise FileNotFoundError("Nenhum arquivo de faixas do Spotify encontrado em data/raw/music/!")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Ingestão de Faixas Musicais (Músicas & Audio Features)[/bold cyan]")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='16GB';")

        # ---------------------------------------------------------------------
        # 1. IDENTIFICAR E INSERIR ARTISTAS INÉDITOS DO SPOTIFY
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Mapeando e cadastrando artistas inéditos do Spotify...[/bold yellow]")
        t0 = time.time()

        if SPOTIFY_PARQUET.exists():
            spotify_source = f"'{SPOTIFY_PARQUET}'"
        else:
            spotify_source = f"read_csv_auto('{SPOTIFY_CSV}')"

        # Isola nomes únicos de artistas primários que não estão na base
        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_new_artists AS
            WITH raw_artists AS (
                SELECT DISTINCT TRIM(SPLIT_PART(artists, ';', 1)) AS name
                FROM {spotify_source}
                WHERE artists IS NOT NULL AND length(TRIM(artists)) > 0
            ),
            dedup_artists AS (
                SELECT name, ROW_NUMBER() OVER (PARTITION BY LOWER(name) ORDER BY length(name) DESC) AS rn
                FROM raw_artists
                WHERE length(name) > 1 AND name NOT LIKE '[%'
            )
            SELECT d.name
            FROM dedup_artists d
            WHERE d.rn = 1
              AND LOWER(d.name) NOT IN (SELECT LOWER(TRIM(name)) FROM music_artists);
            """
        )
        new_artists_count = conn.execute("SELECT COUNT(*) FROM tmp_new_artists").fetchone()[0]
        console.print(f"  -> {new_artists_count:,} novos artistas identificados em {time.time() - t0:.2f}s!")

        if new_artists_count > 0:
            if MB_PARQUET.exists():
                console.print("  -> Cruzando novos artistas com o catálogo MusicBrainz para enriquecer metadados...")
                conn.execute(
                    f"""
                    CREATE OR REPLACE TEMP TABLE tmp_artists_enriched AS
                    WITH mb_dedup AS (
                        SELECT 
                            TRIM(name) AS mb_name,
                            country,
                            type,
                            tags,
                            ROW_NUMBER() OVER (PARTITION BY LOWER(TRIM(name)) ORDER BY len(tags) DESC) AS rn
                        FROM '{MB_PARQUET}'
                        WHERE name IS NOT NULL AND length(TRIM(name)) > 1
                    )
                    SELECT 
                        n.name,
                        mb.country,
                        mb.type,
                        mb.tags
                    FROM tmp_new_artists n
                    LEFT JOIN mb_dedup mb ON LOWER(n.name) = LOWER(mb.mb_name) AND mb.rn = 1;

                    INSERT INTO music_artists (name, country, type)
                    SELECT name, country, type
                    FROM tmp_artists_enriched;
                    """
                )
                # Enriquecer keywords e tags dos novos artistas encontrados no MB
                conn.execute(
                    """
                    CREATE OR REPLACE TEMP TABLE tmp_new_artist_tags AS
                    SELECT 
                        ma.id AS artist_id,
                        LOWER(TRIM(UNNEST(t.tags))) AS tag_name
                    FROM tmp_artists_enriched t
                    JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(t.name)
                    WHERE t.tags IS NOT NULL AND len(t.tags) > 0;

                    INSERT INTO keywords (name)
                    SELECT DISTINCT tag_name FROM tmp_new_artist_tags
                    WHERE tag_name NOT IN (SELECT name FROM keywords)
                      AND length(tag_name) > 1 AND length(tag_name) < 50;

                    INSERT INTO artist_tags (artist_id, keyword_id, weight)
                    SELECT DISTINCT t.artist_id, k.id, 1.0
                    FROM tmp_new_artist_tags t
                    JOIN keywords k ON k.name = t.tag_name
                    ON CONFLICT DO NOTHING;
                    """
                )
            else:
                conn.execute(
                    """
                    INSERT INTO music_artists (name)
                    SELECT name FROM tmp_new_artists;
                    """
                )
            total_artists = conn.execute("SELECT COUNT(*) FROM music_artists").fetchone()[0]
            console.print(f"  -> Artistas cadastrados! Total agora em 'music_artists': {total_artists:,}")

        # ---------------------------------------------------------------------
        # 2. IDENTIFICAR E INSERIR ÁLBUNS INÉDITOS DO SPOTIFY
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]2/4 Cadastrando novos álbuns no DuckDB...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_new_albums AS
            WITH raw_albums AS (
                SELECT DISTINCT
                    ma.id AS artist_id,
                    TRIM(t.album_name) AS title
                FROM {spotify_source} t
                JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(SPLIT_PART(t.artists, ';', 1)))
                WHERE t.album_name IS NOT NULL 
                  AND length(TRIM(t.album_name)) > 0
                  AND t.album_name NOT IN ('null', 'None', '[unknown]')
            ),
            dedup_albums AS (
                SELECT artist_id, title, ROW_NUMBER() OVER (PARTITION BY artist_id, LOWER(title) ORDER BY length(title) DESC) AS rn
                FROM raw_albums
            )
            SELECT d.artist_id, d.title
            FROM dedup_albums d
            WHERE d.rn = 1
              AND NOT EXISTS (
                  SELECT 1 FROM music_albums alb 
                  WHERE alb.artist_id = d.artist_id AND LOWER(TRIM(alb.title)) = LOWER(d.title)
              );

            INSERT INTO music_albums (artist_id, title)
            SELECT artist_id, title FROM tmp_new_albums;
            """
        )
        total_albums = conn.execute("SELECT COUNT(*) FROM music_albums").fetchone()[0]
        console.print(f"  -> Álbuns atualizados em {time.time() - t0:.2f}s! Total em 'music_albums': {total_albums:,}")

        # ---------------------------------------------------------------------
        # 3. MAPEAMENTO E INSERÇÃO DAS FAIXAS COM AUDIO FEATURES
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]3/4 Inserindo faixas musicais e métricas de áudio...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_resolved_tracks AS
            WITH distinct_spotify AS (
                SELECT 
                    track_id,
                    TRIM(track_name) AS track_title,
                    TRIM(SPLIT_PART(artists, ';', 1)) AS primary_artist,
                    TRIM(album_name) AS album_title,
                    CAST(duration_ms AS BIGINT) AS duration_ms,
                    danceability,
                    energy,
                    tempo,
                    valence,
                    acousticness,
                    instrumentalness,
                    ROW_NUMBER() OVER (PARTITION BY track_id ORDER BY duration_ms DESC) as rn
                FROM {spotify_source}
                WHERE track_id IS NOT NULL 
                  AND track_name IS NOT NULL 
                  AND length(TRIM(track_name)) > 0
            )
            SELECT 
                t.track_id AS spotify_id,
                ma.id AS artist_id,
                alb.id AS album_id,
                t.track_title AS title,
                t.duration_ms,
                0 AS popularity,
                t.danceability,
                t.energy,
                t.tempo,
                t.valence,
                t.acousticness,
                t.instrumentalness
            FROM distinct_spotify t
            JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(t.primary_artist))
            LEFT JOIN music_albums alb ON alb.artist_id = ma.id AND LOWER(TRIM(alb.title)) = LOWER(TRIM(t.album_title))
            WHERE t.rn = 1;
            """
        )

        resolved_count = conn.execute("SELECT COUNT(*) FROM tmp_resolved_tracks").fetchone()[0]
        console.print(f"  -> {resolved_count:,} faixas mapeadas com integridade relacional em {time.time() - t0:.2f}s!")

        t1 = time.time()
        conn.execute(
            """
            INSERT INTO music_tracks (
                spotify_id, artist_id, album_id, title, duration_ms, popularity,
                danceability, energy, tempo, valence, acousticness, instrumentalness
            )
            SELECT 
                spotify_id, artist_id, album_id, title, duration_ms, popularity,
                danceability, energy, tempo, valence, acousticness, instrumentalness
            FROM tmp_resolved_tracks
            ON CONFLICT (spotify_id) DO NOTHING;
            """
        )

        total_tracks = conn.execute("SELECT COUNT(*) FROM music_tracks").fetchone()[0]
        console.print(f"  -> Tabela 'music_tracks' populada com {total_tracks:,} faixas em {time.time() - t1:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de Música finalizada com sucesso em {total_time:.2f} segundos![/bold green]\n")

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
    run_tracks_bulk_ingestion()
