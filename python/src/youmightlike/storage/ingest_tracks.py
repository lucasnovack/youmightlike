from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

SPOTIFY_CSV = Path("data/raw/music/spotify_tracks.csv")


def run_tracks_bulk_ingestion() -> None:
    """Ingere faixas musicais individuais com métricas de áudio conectando a artistas e álbuns no DuckDB."""
    if not SPOTIFY_CSV.exists():
        raise FileNotFoundError(f"Arquivo de faixas não encontrado: {SPOTIFY_CSV}")

    db = init_db()
    console.print("\n[bold cyan]🎵 Iniciando Ingestão de Faixas Musicais (Músicas & Audio Features)[/bold cyan]")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. MAPEAMENTO DE FAIXAS COM ARTISTAS E ÁLBUNS
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]🔍 1/3 Vinculando faixas aos artistas canônicos e álbuns...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_resolved_tracks AS
            WITH raw_tracks AS (
                SELECT 
                    track_id,
                    TRIM(track_name) AS track_title,
                    TRIM(SPLIT_PART(artists, ';', 1)) AS primary_artist,
                    TRIM(album_name) AS album_title,
                    popularity,
                    duration_ms,
                    danceability,
                    energy,
                    tempo,
                    valence,
                    acousticness,
                    instrumentalness
                FROM read_csv_auto('{SPOTIFY_CSV}')
                WHERE track_id IS NOT NULL AND track_name IS NOT NULL AND TRIM(track_name) != ''
            )
            SELECT DISTINCT ON (t.track_id)
                t.track_id AS spotify_id,
                ma.id AS artist_id,
                alb.id AS album_id,
                t.track_title AS title,
                t.duration_ms,
                t.popularity,
                t.danceability,
                t.energy,
                t.tempo,
                t.valence,
                t.acousticness,
                t.instrumentalness
            FROM raw_tracks t
            JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(t.primary_artist))
            LEFT JOIN music_albums alb ON alb.artist_id = ma.id AND LOWER(TRIM(alb.title)) = LOWER(TRIM(t.album_title));
            """
        )

        tracks_count = conn.execute("SELECT COUNT(*) FROM tmp_resolved_tracks").fetchone()[0]
        console.print(f"  -> {tracks_count:,} faixas musicais únicas mapeadas com sucesso em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 2. INSERÇÃO NA TABELA CANÔNICA 'music_tracks'
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]💿 2/3 Inserindo faixas e features acústicas no DuckDB...[/bold magenta]")
        t0 = time.time()

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

        total_tracks_db = conn.execute("SELECT COUNT(*) FROM music_tracks").fetchone()[0]
        console.print(f"  -> Tabela 'music_tracks' populada com {total_tracks_db:,} faixas em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]✨ Ingestão de Faixas Musicais concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

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
