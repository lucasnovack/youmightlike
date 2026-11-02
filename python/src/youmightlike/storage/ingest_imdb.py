from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

RAW_DIR = Path("data/raw/imdb")


def run_imdb_bulk_ingestion(min_votes: int = 2500) -> None:
    """Ingere filmes, notas, diretores e roteiristas dos arquivos bulk do IMDb direto para o DuckDB."""
    db = init_db()

    basics_path = RAW_DIR / "title.basics.tsv.gz"
    ratings_path = RAW_DIR / "title.ratings.tsv.gz"
    crew_path = RAW_DIR / "title.crew.tsv.gz"
    names_path = RAW_DIR / "name.basics.tsv.gz"

    for p in [basics_path, ratings_path, crew_path, names_path]:
        if not p.exists():
            raise FileNotFoundError(f"Arquivo bulk não encontrado: {p}")

    console.print("\n[bold cyan]Iniciando Ingestão em Massa do IMDb via DuckDB[/bold cyan]")
    console.print(f"️ Critério de corte: Filmes com [bold green]>={min_votes}[/bold green] votos no IMDb")

    start_total = time.time()

    with db.connect() as conn:
        # Habilitar multi-threading máximo no DuckDB
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. FILMES (title.basics + title.ratings)
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Filtrando e inserindo filmes relevantes...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_filtered_movies AS
            SELECT 
                b.tconst AS imdb_id,
                b.primaryTitle AS title,
                b.originalTitle AS original_title,
                TRY_CAST(b.startYear AS INTEGER) AS release_year,
                TRY_CAST(b.runtimeMinutes AS INTEGER) AS runtime,
                r.averageRating AS vote_average,
                r.numVotes AS vote_count,
                b.genres AS genres_str
            FROM read_csv('{basics_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='', types={{'isAdult': 'VARCHAR'}}) b
            JOIN read_csv('{ratings_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') r 
              ON b.tconst = r.tconst
            WHERE b.titleType = 'movie'
              AND r.numVotes >= {min_votes}
              AND b.startYear IS NOT NULL;
            """
        )

        movie_count = conn.execute("SELECT COUNT(*) FROM tmp_filtered_movies").fetchone()[0]
        console.print(f"  -> {movie_count:,} filmes qualificados encontrados em {time.time() - t0:.2f}s!")

        # Inserir na tabela canônica 'movies'
        conn.execute(
            """
            INSERT INTO movies (wikidata_id, imdb_id, title, original_title, release_year, runtime, vote_average, vote_count)
            SELECT 'imdb:' || imdb_id, imdb_id, title, original_title, release_year, runtime, vote_average, vote_count
            FROM tmp_filtered_movies
            ON CONFLICT (imdb_id) DO UPDATE SET
                vote_average = EXCLUDED.vote_average,
                vote_count = EXCLUDED.vote_count,
                runtime = EXCLUDED.runtime;
            """
        )
        console.print(f"  -> Tabela 'movies' atualizada com sucesso.")

        # ---------------------------------------------------------------------
        # 2. GÊNEROS (genres & movie_genres)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]️ 2/4 Mapeando gêneros cinematográficos...[/bold blue]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_movie_genres_flat AS
            SELECT 
                m.id AS movie_id,
                TRIM(LOWER(UNNEST(string_split(tmp.genres_str, ',')))) AS genre_name
            FROM tmp_filtered_movies tmp
            JOIN movies m ON tmp.imdb_id = m.imdb_id
            WHERE tmp.genres_str IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO genres (name)
            SELECT DISTINCT genre_name FROM tmp_movie_genres_flat
            WHERE genre_name IS NOT NULL AND genre_name != ''
            ON CONFLICT DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO movie_genres (movie_id, genre_id)
            SELECT DISTINCT f.movie_id, g.id
            FROM tmp_movie_genres_flat f
            JOIN genres g ON f.genre_name = g.name
            ON CONFLICT DO NOTHING;
            """
        )
        console.print(f"  -> Gêneros vinculados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. DIRETORES (title.crew + name.basics)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]3/4 Extraindo diretores e montando créditos...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_directors_raw AS
            SELECT 
                m.id AS movie_id,
                UNNEST(string_split(c.directors, ',')) AS nconst
            FROM read_csv('{crew_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') c
            JOIN movies m ON c.tconst = m.imdb_id
            WHERE c.directors IS NOT NULL;
            """
        )

        # Buscar nomes dos diretores que existem na nossa base
        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_directors_named AS
            SELECT DISTINCT
                d.movie_id,
                d.nconst AS imdb_id,
                n.primaryName AS name
            FROM tmp_directors_raw d
            JOIN read_csv('{names_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') n
              ON d.nconst = n.nconst
            WHERE n.primaryName IS NOT NULL;
            """
        )

        # Inserir Diretores em 'people'
        conn.execute(
            """
            INSERT INTO people (wikidata_id, imdb_id, name)
            SELECT DISTINCT 'imdb:' || imdb_id, imdb_id, name FROM tmp_directors_named
            ON CONFLICT (imdb_id) DO UPDATE SET name = EXCLUDED.name;
            """
        )

        # Inserir em 'movie_credits'
        conn.execute(
            """
            INSERT INTO movie_credits (movie_id, person_id, job, department)
            SELECT DISTINCT d.movie_id, p.id, 'Director', 'Directing'
            FROM tmp_directors_named d
            JOIN people p ON d.imdb_id = p.imdb_id
            WHERE NOT EXISTS (
                SELECT 1 FROM movie_credits mc 
                WHERE mc.movie_id = d.movie_id AND mc.person_id = p.id AND mc.job = 'Director'
            );
            """
        )
        console.print(f"  -> Diretores processados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 4. ROTEIRISTAS (title.crew + name.basics)
        # ---------------------------------------------------------------------
        console.print("\n[bold cyan]️ 4/4 Extraindo roteiristas...[/bold cyan]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_writers_raw AS
            SELECT 
                m.id AS movie_id,
                UNNEST(string_split(c.writers, ',')) AS nconst
            FROM read_csv('{crew_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') c
            JOIN movies m ON c.tconst = m.imdb_id
            WHERE c.writers IS NOT NULL;
            """
        )

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_writers_named AS
            SELECT DISTINCT
                w.movie_id,
                w.nconst AS imdb_id,
                n.primaryName AS name
            FROM tmp_writers_raw w
            JOIN read_csv('{names_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') n
              ON w.nconst = n.nconst
            WHERE n.primaryName IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO people (wikidata_id, imdb_id, name)
            SELECT DISTINCT 'imdb:' || imdb_id, imdb_id, name FROM tmp_writers_named
            ON CONFLICT (imdb_id) DO UPDATE SET name = EXCLUDED.name;
            """
        )

        conn.execute(
            """
            INSERT INTO movie_credits (movie_id, person_id, job, department)
            SELECT DISTINCT w.movie_id, p.id, 'Screenplay', 'Writing'
            FROM tmp_writers_named w
            JOIN people p ON w.imdb_id = p.imdb_id
            WHERE NOT EXISTS (
                SELECT 1 FROM movie_credits mc 
                WHERE mc.movie_id = w.movie_id AND mc.person_id = p.id AND mc.job = 'Screenplay'
            );
            """
        )
        console.print(f"  -> Roteiristas processados em {time.time() - t0:.2f}s!")

        total_elapsed = time.time() - start_total
        console.print(f"\n[bold green]Ingestão Bulk Finalizada com Sucesso em {total_elapsed:.2f}s![/bold green]\n")

        # Exibir Estatísticas Finais
        stats_table = Table(title="Estatísticas Atualizadas do DuckDB", header_style="bold green")
        stats_table.add_column("Tabela / View", style="cyan")
        stats_table.add_column("Total de Registros", justify="right", style="bold white")

        for entity in [
            "movies", "people", "movie_credits", "genres", "movie_genres",
            "v_graph_nodes", "v_graph_edges", "v_weighted_graph_edges"
        ]:
            c = conn.execute(f"SELECT COUNT(*) FROM {entity}").fetchone()[0]
            stats_table.add_row(entity, f"{c:,}")

        console.print(stats_table)


if __name__ == "__main__":
    run_imdb_bulk_ingestion(min_votes=2500)
