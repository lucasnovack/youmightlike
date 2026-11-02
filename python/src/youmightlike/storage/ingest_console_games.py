from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

CONSOLE_PARQUET = Path("data/raw/games/console_games.parquet")


def run_console_games_ingestion(min_sales: float = 0.05) -> None:
    """Ingere jogos clássicos de consoles (Nintendo, PlayStation, Xbox, Sega) evitando duplicações com a Steam."""
    if not CONSOLE_PARQUET.exists():
        raise FileNotFoundError(f"Arquivo de jogos de console não encontrado: {CONSOLE_PARQUET}")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Ingestão de Clássicos de Console (Nintendo, PlayStation, Sega)[/bold cyan]")
    console.print(f"️ Critério de corte: Jogos com vendas globais notáveis (>= {min_sales * 1_000_000:,.0f} cópias)")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. CONSOLIDAR JOGOS POR TÍTULO E VERIFICAR DUPLICATAS COM A STEAM
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Consolidando títulos multiplataforma e verificando catálogo Steam...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_console_consolidated AS
            SELECT 
                TRIM(Name) AS name,
                MIN(Rank) AS rank,
                MIN(TRY_CAST(Year AS INTEGER)) AS release_year,
                ARBITRARY(TRIM(Genre)) AS genre,
                ARBITRARY(TRIM(Publisher)) AS publisher,
                SUM(COALESCE(Global_Sales, 0)) AS total_sales,
                STRING_AGG(DISTINCT Platform, ', ') AS platforms_str,
                ARRAY_AGG(DISTINCT Platform) AS platforms_arr
            FROM '{CONSOLE_PARQUET}'
            WHERE Name IS NOT NULL AND TRIM(Name) != ''
            GROUP BY TRIM(Name)
            HAVING SUM(COALESCE(Global_Sales, 0)) >= {min_sales};
            """
        )

        total_console_titles = conn.execute("SELECT COUNT(*) FROM tmp_console_consolidated").fetchone()[0]
        console.print(f"  -> {total_console_titles:,} títulos clássicos qualificados em {time.time() - t0:.2f}s!")

        # Mapear jogos já existentes (via Steam)
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_console_matching AS
            SELECT 
                c.*,
                g.id AS existing_game_id
            FROM tmp_console_consolidated c
            LEFT JOIN games g ON LOWER(TRIM(g.title)) = LOWER(TRIM(c.name));
            """
        )

        matched_steam = conn.execute("SELECT COUNT(*) FROM tmp_console_matching WHERE existing_game_id IS NOT NULL").fetchone()[0]
        new_console_count = total_console_titles - matched_steam
        console.print(f"  -> {matched_steam:,} jogos já existiam no banco (Steam) | {new_console_count:,} clássicos exclusivos de console para inserção.")

        # Inserir novos jogos exclusivos de console
        # Gerando app_id sintético fora do range da Steam: 2_000_000_000 + rank
        conn.execute(
            """
            INSERT INTO games (app_id, title, release_year, publishers, developers, positive_reviews)
            SELECT 
                (2000000000 + rank) AS app_id,
                name AS title,
                release_year,
                publisher AS publishers,
                publisher AS developers,
                CAST(total_sales * 10000 AS BIGINT) AS positive_reviews
            FROM tmp_console_matching
            WHERE existing_game_id IS NULL
            ON CONFLICT (app_id) DO NOTHING;
            """
        )

        # Consolidar todos os game_ids resolvidos (existentes + recém criados)
        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_console_resolved AS
            SELECT 
                COALESCE(m.existing_game_id, g.id) AS game_id,
                m.name,
                m.publisher,
                m.genre,
                m.platforms_arr
            FROM tmp_console_matching m
            JOIN games g ON LOWER(TRIM(g.title)) = LOWER(TRIM(m.name));
            """
        )

        total_games_now = conn.execute("SELECT COUNT(*) FROM games").fetchone()[0]
        console.print(f"  -> Catálogo unificado de jogos expandido para {total_games_now:,} títulos!")

        # ---------------------------------------------------------------------
        # 2. ESTÚDIOS E PUBLICADORAS (game_developers & game_dev_links)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]2/4 Mapeando estúdios e publicadoras de console (developed_by)...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            """
            INSERT INTO game_developers (name)
            SELECT DISTINCT publisher
            FROM tmp_console_resolved
            WHERE publisher IS NOT NULL AND publisher != '' AND length(publisher) > 1
            ON CONFLICT DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO game_dev_links (game_id, developer_id)
            SELECT DISTINCT
                r.game_id,
                gd.id AS developer_id
            FROM tmp_console_resolved r
            JOIN game_developers gd ON LOWER(TRIM(gd.name)) = LOWER(TRIM(r.publisher))
            WHERE r.publisher IS NOT NULL AND r.publisher != ''
            ON CONFLICT (game_id, developer_id) DO NOTHING;
            """
        )

        dev_links_count = conn.execute("SELECT COUNT(*) FROM game_dev_links").fetchone()[0]
        console.print(f"  -> Total de {dev_links_count:,} arestas de desenvolvimento registradas em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. GÊNEROS E PLATAFORMAS (keywords & game_tags)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]️ 3/4 Mapeando gêneros e plataformas de console (has_tag)...[/bold green]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_console_tags_flat AS
            SELECT 
                r.game_id,
                LOWER(TRIM(r.genre)) AS tag_name
            FROM tmp_console_resolved r
            WHERE r.genre IS NOT NULL AND TRIM(r.genre) != ''
            UNION ALL
            SELECT 
                r.game_id,
                LOWER(TRIM('console-' || UNNEST(r.platforms_arr))) AS tag_name
            FROM tmp_console_resolved r
            WHERE r.platforms_arr IS NOT NULL;
            """
        )

        # Inserir novas tags em keywords
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'console_games'
            FROM tmp_console_tags_flat
            WHERE length(tag_name) >= 2
            ON CONFLICT (name) DO NOTHING;
            """
        )

        # Vincular game_tags
        conn.execute(
            """
            INSERT INTO game_tags (game_id, keyword_id)
            SELECT DISTINCT
                f.game_id,
                k.id AS keyword_id
            FROM tmp_console_tags_flat f
            JOIN keywords k ON k.name = f.tag_name
            WHERE length(f.tag_name) >= 2
            ON CONFLICT (game_id, keyword_id) DO NOTHING;
            """
        )

        game_tags_count = conn.execute("SELECT COUNT(*) FROM game_tags").fetchone()[0]
        total_keywords = conn.execute("SELECT COUNT(*) FROM keywords").fetchone()[0]
        console.print(f"  -> Total de {game_tags_count:,} arestas de tags em jogos ({total_keywords:,} keywords globais) em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de Clássicos de Console concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

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
    run_console_games_ingestion()
