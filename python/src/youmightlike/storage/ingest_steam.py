from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

PARQUET_FILE = Path("data/raw/steam/games.parquet")


def run_steam_bulk_ingestion(min_reviews: int = 400) -> None:
    """Ingere jogos, desenvolvedoras e gêneros da Steam diretamente do arquivo Parquet no DuckDB."""
    if not PARQUET_FILE.exists():
        raise FileNotFoundError(f"Arquivo Parquet não encontrado: {PARQUET_FILE}")

    db = init_db()
    console.print("\n[bold cyan]🚀 Iniciando Ingestão em Massa da Steam via DuckDB (Parquet Nativo)[/bold cyan]")
    console.print(f"⚙️ Critério de corte: Jogos com [bold green]>={min_reviews}[/bold green] avaliações na Steam")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. FILTRAR JOGOS QUALIFICADOS
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]🎮 1/3 Filtrando jogos notáveis...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_filtered_games AS
            SELECT 
                TRY_CAST(appID AS BIGINT) AS app_id,
                TRIM(name) AS title,
                release_date,
                TRY_CAST(REGEXP_EXTRACT(release_date, '[0-9]{{4}}') AS INTEGER) AS release_year,
                price,
                positive AS positive_reviews,
                negative AS negative_reviews,
                ARRAY_TO_STRING(developers, ', ') AS developers_str,
                ARRAY_TO_STRING(publishers, ', ') AS publishers,
                developers,
                ARRAY_CONCAT(COALESCE(genres, []), COALESCE(categories, [])) AS tags_arr
            FROM '{PARQUET_FILE}'
            WHERE TRY_CAST(appID AS BIGINT) IS NOT NULL
              AND name IS NOT NULL AND TRIM(name) != ''
              AND (COALESCE(positive, 0) + COALESCE(negative, 0)) >= {min_reviews};
            """
        )

        game_count = conn.execute("SELECT COUNT(*) FROM tmp_filtered_games").fetchone()[0]
        console.print(f"  -> {game_count:,} jogos qualificados encontrados em {time.time() - t0:.2f}s!")

        # Inserir na tabela canônica 'games'
        conn.execute(
            """
            INSERT INTO games (app_id, title, release_date, release_year, price, positive_reviews, negative_reviews, developers, publishers)
            SELECT app_id, title, release_date, release_year, price, positive_reviews, negative_reviews, developers_str, publishers
            FROM tmp_filtered_games
            ON CONFLICT (app_id) DO UPDATE SET
                positive_reviews = EXCLUDED.positive_reviews,
                negative_reviews = EXCLUDED.negative_reviews,
                price = EXCLUDED.price;
            """
        )
        console.print(f"  -> Tabela 'games' populada.")

        # ---------------------------------------------------------------------
        # 2. DESENVOLVEDORES (game_developers & game_dev_links)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]🏢 2/3 Mapeando estúdios desenvolvedores...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_game_devs_flat AS
            SELECT 
                g.id AS game_id,
                TRIM(UNNEST(tmp.developers)) AS dev_name
            FROM tmp_filtered_games tmp
            JOIN games g ON tmp.app_id = g.app_id
            WHERE tmp.developers IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO game_developers (name)
            SELECT DISTINCT dev_name FROM tmp_game_devs_flat
            WHERE dev_name != '' AND dev_name IS NOT NULL
            ON CONFLICT DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO game_dev_links (game_id, developer_id)
            SELECT DISTINCT f.game_id, d.id
            FROM tmp_game_devs_flat f
            JOIN game_developers d ON f.dev_name = d.name
            ON CONFLICT DO NOTHING;
            """
        )
        console.print(f"  -> Estúdios e vínculos processados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. TAGS ESTÉTICAS (keywords & game_tags)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]🏷️ 3/3 Vinculando tags estéticas e temáticas aos jogos...[/bold blue]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_game_tags_flat AS
            SELECT 
                g.id AS game_id,
                TRIM(LOWER(UNNEST(tmp.tags_arr))) AS tag_name
            FROM tmp_filtered_games tmp
            JOIN games g ON tmp.app_id = g.app_id
            WHERE tmp.tags_arr IS NOT NULL;
            """
        )

        # Inserir tags únicas em keywords
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'steam'
            FROM tmp_game_tags_flat
            WHERE tag_name != '' AND tag_name IS NOT NULL
            ON CONFLICT DO NOTHING;
            """
        )

        # Vincular em game_tags
        conn.execute(
            """
            INSERT INTO game_tags (game_id, keyword_id)
            SELECT DISTINCT f.game_id, k.id
            FROM tmp_game_tags_flat f
            JOIN keywords k ON f.tag_name = k.name
            ON CONFLICT DO NOTHING;
            """
        )
        console.print(f"  -> Tags estéticas vinculadas em {time.time() - t0:.2f}s!")

        total_elapsed = time.time() - start_total
        console.print(f"\n[bold green]🏁 Ingestão da Steam Finalizada com Sucesso em {total_elapsed:.2f}s![/bold green]\n")

        # Exibir Estatísticas Finais
        stats_table = Table(title="📈 Estatísticas Atualizadas do Grafo (Com Games)", header_style="bold green")
        stats_table.add_column("Tabela / View", style="cyan")
        stats_table.add_column("Total de Registros", justify="right", style="bold white")

        for entity in [
            "movies", "people", "books", "music_artists", "music_albums",
            "games", "game_developers", "keywords",
            "v_graph_nodes", "v_graph_edges", "v_weighted_graph_edges"
        ]:
            c = conn.execute(f"SELECT COUNT(*) FROM {entity}").fetchone()[0]
            stats_table.add_row(entity, f"{c:,}")

        console.print(stats_table)


if __name__ == "__main__":
    run_steam_bulk_ingestion(min_reviews=400)
