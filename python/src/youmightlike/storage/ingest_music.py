from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

MB_PARQUET = Path("data/raw/music/musicbrainz_artists.parquet")
SPOTIFY_CSV = Path("data/raw/music/spotify_tracks.csv")


def run_music_bulk_ingestion(min_tags: int = 2) -> None:
    """Ingere artistas, tags estéticas e álbuns do MusicBrainz e Spotify no DuckDB."""
    if not MB_PARQUET.exists():
        raise FileNotFoundError(f"Arquivo MusicBrainz Parquet não encontrado: {MB_PARQUET}")
    if not SPOTIFY_CSV.exists():
        raise FileNotFoundError(f"Arquivo Spotify CSV não encontrado: {SPOTIFY_CSV}")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Ingestão em Massa de Música via DuckDB (MusicBrainz + Spotify)[/bold cyan]")
    console.print(f"️ Critério de corte: Artistas MusicBrainz com [bold green]>={min_tags}[/bold green] tags estéticas")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. FILTRAR E INGERIR ARTISTAS DO MUSICBRAINZ
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Mapeando artistas qualificados do MusicBrainz...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_mb_artists AS
            SELECT 
                mb_id AS mbid,
                TRIM(name) AS name,
                country,
                type,
                tags
            FROM '{MB_PARQUET}'
            WHERE len(tags) >= {min_tags}
              AND name NOT LIKE '[%'
              AND name NOT IN ('Various Artists', 'Unknown Artist', 'Chorus', 'Traditional')
              AND length(TRIM(name)) > 1;
            """
        )

        mb_count = conn.execute("SELECT COUNT(*) FROM tmp_mb_artists").fetchone()[0]
        console.print(f"  -> {mb_count:,} artistas qualificados selecionados do MusicBrainz em {time.time() - t0:.2f}s!")

        # Inserir artistas na tabela canônica 'music_artists'
        conn.execute(
            """
            INSERT INTO music_artists (mbid, name, country, type)
            SELECT mbid, name, country, type
            FROM tmp_mb_artists
            ON CONFLICT (mbid) DO UPDATE SET
                country = COALESCE(music_artists.country, EXCLUDED.country),
                type = COALESCE(music_artists.type, EXCLUDED.type);
            """
        )
        console.print(f"  -> Artistas do MusicBrainz inseridos com sucesso.")

        # ---------------------------------------------------------------------
        # 2. ARTISTAS DO SPOTIFY (COMPLEMENTARES)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]2/4 Complementando com artistas do Spotify...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_sp_artists AS
            SELECT 
                'sp:' || md5(lower(TRIM(artists))) AS mbid,
                TRIM(artists) AS name,
                'Artist' AS type,
                MAX(popularity) AS max_popularity,
                ARBITRARY(track_genre) AS genre
            FROM read_csv_auto('{SPOTIFY_CSV}')
            WHERE artists IS NOT NULL AND TRIM(artists) != ''
            GROUP BY 1, 2;
            """
        )

        # Inserir artistas do Spotify cujo nome ainda não exista
        conn.execute(
            """
            INSERT INTO music_artists (mbid, name, type)
            SELECT sp.mbid, sp.name, sp.type
            FROM tmp_sp_artists sp
            WHERE NOT EXISTS (
                SELECT 1 FROM music_artists ma WHERE LOWER(TRIM(ma.name)) = LOWER(TRIM(sp.name))
            )
            ON CONFLICT (mbid) DO NOTHING;
            """
        )

        total_artists = conn.execute("SELECT COUNT(*) FROM music_artists").fetchone()[0]
        console.print(f"  -> Total consolidado de artistas: {total_artists:,} em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. TAGS ESTÉTICAS & GÊNEROS (keywords & artist_tags)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]️ 3/4 Mapeando tags estéticas e de gênero (has_tag)...[/bold green]")
        t0 = time.time()

        # Desaninhando tags do MusicBrainz
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_mb_tags_flat AS
            SELECT 
                ma.id AS artist_id,
                LOWER(TRIM(UNNEST(mb.tags))) AS tag_name
            FROM tmp_mb_artists mb
            JOIN music_artists ma ON ma.mbid = mb.mbid
            WHERE mb.tags IS NOT NULL;
            """
        )

        # Tags/gêneros do Spotify
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_sp_tags_flat AS
            SELECT 
                ma.id AS artist_id,
                LOWER(TRIM(sp.genre)) AS tag_name
            FROM tmp_sp_artists sp
            JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(sp.name))
            WHERE sp.genre IS NOT NULL AND TRIM(sp.genre) != '';
            """
        )

        # Inserir tags em 'keywords'
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'musicbrainz'
            FROM tmp_mb_tags_flat
            WHERE length(tag_name) >= 2
            ON CONFLICT (name) DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'spotify'
            FROM tmp_sp_tags_flat
            WHERE length(tag_name) >= 2
            ON CONFLICT (name) DO NOTHING;
            """
        )

        # Vincular artist_tags (MusicBrainz)
        conn.execute(
            """
            INSERT INTO artist_tags (artist_id, keyword_id, weight)
            SELECT DISTINCT
                f.artist_id,
                k.id AS keyword_id,
                1.0 AS weight
            FROM tmp_mb_tags_flat f
            JOIN keywords k ON k.name = f.tag_name
            WHERE length(f.tag_name) >= 2
            ON CONFLICT (artist_id, keyword_id) DO NOTHING;
            """
        )

        # Vincular artist_tags (Spotify)
        conn.execute(
            """
            INSERT INTO artist_tags (artist_id, keyword_id, weight)
            SELECT DISTINCT
                f.artist_id,
                k.id AS keyword_id,
                1.0 AS weight
            FROM tmp_sp_tags_flat f
            JOIN keywords k ON k.name = f.tag_name
            WHERE length(f.tag_name) >= 2
            ON CONFLICT (artist_id, keyword_id) DO NOTHING;
            """
        )

        tags_count = conn.execute("SELECT COUNT(*) FROM artist_tags").fetchone()[0]
        keywords_count = conn.execute("SELECT COUNT(*) FROM keywords").fetchone()[0]
        console.print(f"  -> {tags_count:,} arestas de tags musicais (com {keywords_count:,} keywords globais) em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 4. ÁLBUNS MUSICAIS (music_albums)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]4/4 Mapeando álbuns musicais (released)...[/bold blue]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_albums_grouped AS
            SELECT 
                'sp:' || md5(lower(TRIM(artists)) || '::' || lower(TRIM(album_name))) AS album_mbid,
                TRIM(artists) AS artist_name,
                TRIM(album_name) AS album_title,
                'Album' AS primary_type
            FROM read_csv_auto('{SPOTIFY_CSV}')
            WHERE album_name IS NOT NULL AND TRIM(album_name) != ''
              AND artists IS NOT NULL AND TRIM(artists) != ''
            GROUP BY 1, 2, 3;
            """
        )

        conn.execute(
            """
            INSERT INTO music_albums (mbid, artist_id, title, primary_type)
            SELECT DISTINCT
                alb.album_mbid,
                ma.id AS artist_id,
                alb.album_title,
                alb.primary_type
            FROM tmp_albums_grouped alb
            JOIN music_artists ma ON LOWER(TRIM(ma.name)) = LOWER(TRIM(alb.artist_name))
            ON CONFLICT (mbid) DO NOTHING;
            """
        )

        albums_count = conn.execute("SELECT COUNT(*) FROM music_albums").fetchone()[0]
        console.print(f"  -> {albums_count:,} álbuns cadastrados e conectados aos artistas em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de Música concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

        _print_summary(conn)


def _print_summary(conn) -> None:
    table = Table(title="Censo Geral do Grafo Transmidiático (DuckDB)", show_header=True, header_style="bold cyan")
    table.add_column("Domínio / Tabela", style="bold")
    table.add_column("Total de Registros", justify="right", style="green")

    tables = [
        ("Filmes (movies)", "SELECT count(*) FROM movies"),
        ("Pessoas / Equipe (people)", "SELECT count(*) FROM people"),
        ("Créditos de Filmes (movie_credits)", "SELECT count(*) FROM movie_credits"),
        ("Jogos (games)", "SELECT count(*) FROM games"),
        ("Desenvolvedores (game_developers)", "SELECT count(*) FROM game_developers"),
        ("Tags de Jogos (game_tags)", "SELECT count(*) FROM game_tags"),
        ("Livros (books)", "SELECT count(*) FROM books"),
        ("Autores de Livros (book_authors)", "SELECT count(*) FROM book_authors"),
        ("Relações de Autoria (book_author_links)", "SELECT count(*) FROM book_author_links"),
        ("Temas de Livros (book_subjects)", "SELECT count(*) FROM book_subjects"),
        ("Artistas Musicais (music_artists)", "SELECT count(*) FROM music_artists"),
        ("Álbuns Musicais (music_albums)", "SELECT count(*) FROM music_albums"),
        ("Tags de Artistas (artist_tags)", "SELECT count(*) FROM artist_tags"),
        ("Keywords Globais (keywords)", "SELECT count(*) FROM keywords"),
        ("Nós no Grafo (v_graph_nodes)", "SELECT count(*) FROM v_graph_nodes"),
        ("Arestas Ativas (v_graph_edges)", "SELECT count(*) FROM v_graph_edges"),
    ]

    for name, query in tables:
        count = conn.execute(query).fetchone()[0]
        table.add_row(name, f"{count:,}")

    console.print(table)


if __name__ == "__main__":
    run_music_bulk_ingestion()
