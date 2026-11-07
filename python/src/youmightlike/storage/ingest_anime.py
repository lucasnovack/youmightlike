from __future__ import annotations

import time
from pathlib import Path

from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

ANIME_CSV = Path("data/raw/anime/top_anime.csv")


def run_anime_bulk_ingestion(min_members: int = 20000) -> None:
    """Ingere animes, estúdios de animação e temas estéticos do MyAnimeList conectando ao grafo existente."""
    if not ANIME_CSV.exists():
        raise FileNotFoundError(f"Arquivo MyAnimeList não encontrado: {ANIME_CSV}")

    db = init_db()
    console.print("\n[bold cyan]️ Iniciando Ingestão do MyAnimeList (Animes, Estúdios e Temas)[/bold cyan]")
    console.print(f"️ Critério de corte: Obras com [bold green]>={min_members:,}[/bold green] membros na comunidade")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. FILTRAR ANIMES QUALIFICADOS E EXTRAIR CAMPOS
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Filtrando animes notáveis e mapeando com filmes existentes...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_raw_anime AS
            SELECT 
                REGEXP_EXTRACT(URL, 'anime/([0-9]+)', 1) AS mal_id,
                COALESCE(NULLIF(TRIM(English), ''), TRIM(Title)) AS main_title,
                TRIM(Title) AS romaji_title,
                TRY_CAST(REGEXP_EXTRACT(Aired, '([12][0-9]{{3}})', 1) AS INTEGER) AS release_year,
                TRY_CAST(Score AS DOUBLE) AS score,
                TRY_CAST(REPLACE(Members, ',', '') AS BIGINT) AS members_count,
                Studios,
                Genres,
                Themes
            FROM read_csv_auto('{ANIME_CSV}')
            WHERE REGEXP_EXTRACT(URL, 'anime/([0-9]+)', 1) != ''
              AND TRY_CAST(REPLACE(Members, ',', '') AS BIGINT) >= {min_members};
            """
        )

        anime_count = conn.execute("SELECT COUNT(*) FROM tmp_raw_anime").fetchone()[0]
        console.print(f"  -> {anime_count:,} animes qualificados selecionados em {time.time() - t0:.2f}s!")

        # Mapear quais animes já existem em 'movies'
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_anime_with_db_id AS
            SELECT 
                a.*,
                COALESCE(m_imdb.id, m_en.id, m_ro.id) AS existing_movie_id
            FROM tmp_raw_anime a
            LEFT JOIN movies m_imdb ON m_imdb.wikidata_id = ('mal:' || a.mal_id)
            LEFT JOIN movies m_en ON (m_imdb.id IS NULL AND LOWER(TRIM(m_en.title)) = LOWER(TRIM(a.main_title)))
            LEFT JOIN movies m_ro ON (m_imdb.id IS NULL AND m_en.id IS NULL AND LOWER(TRIM(m_ro.original_title)) = LOWER(TRIM(a.romaji_title)));
            """
        )

        matched_count = conn.execute("SELECT COUNT(*) FROM tmp_anime_with_db_id WHERE existing_movie_id IS NOT NULL").fetchone()[0]
        new_anime_count = anime_count - matched_count
        console.print(f"  -> {matched_count:,} obras já existiam no banco (IMDb/Wikidata) | {new_anime_count:,} animes novos para inserção.")

        # Inserir animes novos em 'movies'
        conn.execute(
            """
            INSERT INTO movies (wikidata_id, title, original_title, release_year, vote_average, vote_count)
            SELECT 
                'mal:' || mal_id AS wikidata_id,
                main_title AS title,
                romaji_title AS original_title,
                release_year,
                score AS vote_average,
                members_count AS vote_count
            FROM tmp_anime_with_db_id
            WHERE existing_movie_id IS NULL
            ON CONFLICT (wikidata_id) DO NOTHING;
            """
        )

        # Consolidar a tabela com todos os movie_ids (existentes + recém inseridos)
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_anime_resolved AS
            SELECT 
                a.mal_id,
                COALESCE(m1.id, m2.id, m3.id) AS movie_id,
                a.Studios,
                a.Genres,
                a.Themes
            FROM tmp_raw_anime a
            LEFT JOIN movies m1 ON m1.wikidata_id = ('mal:' || a.mal_id)
            LEFT JOIN movies m2 ON (m1.id IS NULL AND LOWER(TRIM(m2.title)) = LOWER(TRIM(a.main_title)))
            LEFT JOIN movies m3 ON (m1.id IS NULL AND m2.id IS NULL AND LOWER(TRIM(COALESCE(m3.original_title, ''))) = LOWER(TRIM(a.romaji_title)))
            WHERE COALESCE(m1.id, m2.id, m3.id) IS NOT NULL;
            """
        )
        resolved_count = conn.execute("SELECT COUNT(DISTINCT movie_id) FROM tmp_anime_resolved").fetchone()[0]
        console.print(f"  -> Total de {resolved_count:,} obras de anime integradas ao catálogo de filmes!")

        # ---------------------------------------------------------------------
        # 2. ESTÚDIOS DE ANIMAÇÃO (people & movie_credits)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]2/4 Mapeando estúdios de animação (produced_by)...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_anime_studios AS
            SELECT 
                r.movie_id,
                TRIM(UNNEST(STRING_SPLIT(r.Studios, ','))) AS studio_name
            FROM tmp_anime_resolved r
            WHERE r.Studios IS NOT NULL AND TRIM(r.Studios) != '';
            """
        )

        # Inserir estúdios em 'people' com identificador determinístico
        conn.execute(
            """
            INSERT INTO people (wikidata_id, name)
            SELECT DISTINCT 
                'studio:' || md5(lower(studio_name)) AS wikidata_id,
                studio_name AS name
            FROM tmp_anime_studios
            WHERE studio_name != '' AND length(studio_name) > 1
            ON CONFLICT (wikidata_id) DO NOTHING;
            """
        )

        # Criar créditos de 'produced_by' (Animation Studio)
        conn.execute(
            """
            INSERT INTO movie_credits (movie_id, person_id, job, department)
            SELECT DISTINCT
                s.movie_id,
                p.id AS person_id,
                'Animation Studio' AS job,
                'Production' AS department
            FROM tmp_anime_studios s
            JOIN people p ON p.wikidata_id = ('studio:' || md5(lower(s.studio_name)))
            WHERE s.studio_name != ''
            ON CONFLICT DO NOTHING;
            """
        )

        studios_count = conn.execute("SELECT COUNT(DISTINCT person_id) FROM movie_credits WHERE job = 'Animation Studio'").fetchone()[0]
        studio_edges = conn.execute("SELECT COUNT(*) FROM movie_credits WHERE job = 'Animation Studio'").fetchone()[0]
        console.print(f"  -> {studios_count:,} estúdios únicos mapeados com {studio_edges:,} arestas 'produced_by' em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. GÊNEROS E TEMAS ESTÉTICOS (keywords & movie_keywords)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]️ 3/4 Mapeando temas, tropos e microgêneros (has_keyword)...[/bold green]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_anime_tags AS
            SELECT 
                r.movie_id,
                LOWER(TRIM(UNNEST(ARRAY_CONCAT(
                    STRING_SPLIT(COALESCE(r.Genres, ''), ','),
                    STRING_SPLIT(COALESCE(r.Themes, ''), ',')
                )))) AS tag_name
            FROM tmp_anime_resolved r;
            """
        )

        # Inserir novas tags em keywords
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'myanimelist'
            FROM tmp_anime_tags
            WHERE length(tag_name) >= 2
            ON CONFLICT (name) DO NOTHING;
            """
        )

        # Conectar animes a keywords
        conn.execute(
            """
            INSERT INTO movie_keywords (movie_id, keyword_id)
            SELECT DISTINCT
                t.movie_id,
                k.id AS keyword_id
            FROM tmp_anime_tags t
            JOIN keywords k ON k.name = t.tag_name
            WHERE length(t.tag_name) >= 2
            ON CONFLICT (movie_id, keyword_id) DO NOTHING;
            """
        )

        anime_tags_count = conn.execute("SELECT COUNT(*) FROM movie_keywords").fetchone()[0]
        total_keywords = conn.execute("SELECT COUNT(*) FROM keywords").fetchone()[0]
        console.print(f"  -> Total de {anime_tags_count:,} arestas de keywords em filmes/animes ({total_keywords:,} keywords globais) em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão do MyAnimeList concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

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
        ("Tags de Jogos (game_tags)", "SELECT count(*) FROM game_tags"),
        ("Livros (books)", "SELECT count(*) FROM books"),
        ("Temas de Livros (book_subjects)", "SELECT count(*) FROM book_subjects"),
        ("Artistas Musicais (music_artists)", "SELECT count(*) FROM music_artists"),
        ("Tags de Música (artist_tags)", "SELECT count(*) FROM artist_tags"),
        ("Similaridades entre Artistas (artist_similarities)", "SELECT count(*) FROM artist_similarities"),
        ("Álbuns Musicais (music_albums)", "SELECT count(*) FROM music_albums"),
        ("Keywords Globais (keywords)", "SELECT count(*) FROM keywords"),
        ("Nós no Grafo (v_graph_nodes)", "SELECT count(*) FROM v_graph_nodes"),
        ("Arestas Ativas (v_graph_edges)", "SELECT count(*) FROM v_graph_edges"),
    ]

    for name, query in tables:
        count = conn.execute(query).fetchone()[0]
        table.add_row(name, f"{count:,}")

    console.print(table)


if __name__ == "__main__":
    run_anime_bulk_ingestion()
