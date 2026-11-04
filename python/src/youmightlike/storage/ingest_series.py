from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

RAW_DIR = Path("data/raw/imdb")


def run_series_bulk_ingestion(min_votes: int = 2500) -> None:
    """Ingere séries e minisséries, diretores, roteiristas, gêneros e pontes transmidiáticas do IMDb direto para o DuckDB."""
    db = init_db()

    basics_path = RAW_DIR / "title.basics.tsv.gz"
    ratings_path = RAW_DIR / "title.ratings.tsv.gz"
    crew_path = RAW_DIR / "title.crew.tsv.gz"
    names_path = RAW_DIR / "name.basics.tsv.gz"

    for p in [basics_path, ratings_path, crew_path, names_path]:
        if not p.exists():
            raise FileNotFoundError(f"Arquivo bulk não encontrado: {p}")

    console.print("\n[bold cyan]Iniciando Ingestão em Massa de Séries & Minisséries do IMDb via DuckDB[/bold cyan]")
    console.print(f"Critério de corte: Séries com [bold green]>={min_votes:,}[/bold green] votos no IMDb")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. SÉRIES & MINISSÉRIES (title.basics + title.ratings)
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/6 Filtrando e inserindo séries notáveis...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_filtered_series AS
            SELECT 
                b.tconst AS imdb_id,
                b.primaryTitle AS title,
                b.originalTitle AS original_title,
                b.titleType AS series_type,
                TRY_CAST(b.startYear AS INTEGER) AS start_year,
                TRY_CAST(b.endYear AS INTEGER) AS end_year,
                TRY_CAST(b.runtimeMinutes AS INTEGER) AS runtime,
                r.averageRating AS vote_average,
                r.numVotes AS vote_count,
                b.genres AS genres_str
            FROM read_csv('{basics_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='', types={{'isAdult': 'VARCHAR'}}) b
            JOIN read_csv('{ratings_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') r 
              ON b.tconst = r.tconst
            WHERE b.titleType IN ('tvSeries', 'tvMiniSeries')
              AND r.numVotes >= {min_votes}
              AND b.startYear IS NOT NULL;
            """
        )

        series_count = conn.execute("SELECT COUNT(*) FROM tmp_filtered_series").fetchone()[0]
        console.print(f"  -> {series_count:,} séries qualificadas encontradas em {time.time() - t0:.2f}s!")

        conn.execute(
            """
            INSERT INTO tv_series (wikidata_id, imdb_id, title, original_title, series_type, start_year, end_year, runtime, vote_average, vote_count)
            SELECT 'imdb:' || imdb_id, imdb_id, title, original_title, series_type, start_year, end_year, runtime, vote_average, vote_count
            FROM tmp_filtered_series
            ON CONFLICT (imdb_id) DO UPDATE SET
                title = EXCLUDED.title,
                original_title = EXCLUDED.original_title,
                series_type = EXCLUDED.series_type,
                start_year = EXCLUDED.start_year,
                end_year = EXCLUDED.end_year,
                vote_average = EXCLUDED.vote_average,
                vote_count = EXCLUDED.vote_count,
                runtime = EXCLUDED.runtime;
            """
        )
        console.print("  -> Tabela 'tv_series' atualizada com sucesso.")

        # ---------------------------------------------------------------------
        # 2. GÊNEROS E TEMAS (genres, keywords, series_genres, series_keywords)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]2/6 Mapeando gêneros e tags estéticas de séries...[/bold blue]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_series_genres_flat AS
            SELECT 
                s.id AS series_id,
                TRIM(LOWER(UNNEST(string_split(tmp.genres_str, ',')))) AS genre_name
            FROM tmp_filtered_series tmp
            JOIN tv_series s ON tmp.imdb_id = s.imdb_id
            WHERE tmp.genres_str IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO genres (name)
            SELECT DISTINCT genre_name FROM tmp_series_genres_flat
            WHERE genre_name IS NOT NULL AND genre_name != ''
            ON CONFLICT DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO series_genres (series_id, genre_id)
            SELECT DISTINCT f.series_id, g.id
            FROM tmp_series_genres_flat f
            JOIN genres g ON f.genre_name = g.name
            ON CONFLICT DO NOTHING;
            """
        )

        # Garantir que todos os gêneros existam no vocabulário unificado 'keywords'
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT genre_name, 'imdb'
            FROM tmp_series_genres_flat
            WHERE genre_name IS NOT NULL AND genre_name != ''
            ON CONFLICT DO NOTHING;
            """
        )

        conn.execute(
            """
            INSERT INTO series_keywords (series_id, keyword_id)
            SELECT DISTINCT f.series_id, k.id
            FROM tmp_series_genres_flat f
            JOIN keywords k ON f.genre_name = k.name
            ON CONFLICT DO NOTHING;
            """
        )

        genre_edge_count = conn.execute("SELECT COUNT(*) FROM series_keywords").fetchone()[0]
        console.print(f"  -> {genre_edge_count:,} arestas de gênero/keyword vinculadas em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. DIRETORES (title.crew + name.basics)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]3/6 Extraindo diretores e showrunners...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_series_directors_raw AS
            SELECT 
                s.id AS series_id,
                UNNEST(string_split(c.directors, ',')) AS nconst
            FROM read_csv('{crew_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') c
            JOIN tv_series s ON c.tconst = s.imdb_id
            WHERE c.directors IS NOT NULL;
            """
        )

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_series_directors_named AS
            SELECT DISTINCT
                d.series_id,
                d.nconst AS imdb_id,
                n.primaryName AS name
            FROM tmp_series_directors_raw d
            JOIN read_csv('{names_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') n
              ON d.nconst = n.nconst
            WHERE n.primaryName IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO people (wikidata_id, imdb_id, name)
            SELECT DISTINCT 'imdb:' || imdb_id, imdb_id, name FROM tmp_series_directors_named
            ON CONFLICT (imdb_id) DO UPDATE SET name = EXCLUDED.name;
            """
        )

        conn.execute(
            """
            INSERT INTO series_credits (series_id, person_id, job, department)
            SELECT DISTINCT d.series_id, p.id, 'Director', 'Directing'
            FROM tmp_series_directors_named d
            JOIN people p ON d.imdb_id = p.imdb_id
            WHERE NOT EXISTS (
                SELECT 1 FROM series_credits sc 
                WHERE sc.series_id = d.series_id AND sc.person_id = p.id AND sc.job = 'Director'
            );
            """
        )
        dir_count = conn.execute("SELECT COUNT(*) FROM series_credits WHERE job = 'Director'").fetchone()[0]
        console.print(f"  -> {dir_count:,} diretores vinculados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 4. ROTEIRISTAS E CRIADORES (title.crew + name.basics)
        # ---------------------------------------------------------------------
        console.print("\n[bold cyan]4/6 Extraindo roteiristas e criadores...[/bold cyan]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_series_writers_raw AS
            SELECT 
                s.id AS series_id,
                UNNEST(string_split(c.writers, ',')) AS nconst
            FROM read_csv('{crew_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') c
            JOIN tv_series s ON c.tconst = s.imdb_id
            WHERE c.writers IS NOT NULL;
            """
        )

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_series_writers_named AS
            SELECT DISTINCT
                w.series_id,
                w.nconst AS imdb_id,
                n.primaryName AS name
            FROM tmp_series_writers_raw w
            JOIN read_csv('{names_path}', delim='\\t', header=TRUE, nullstr='\\N', quote='') n
              ON w.nconst = n.nconst
            WHERE n.primaryName IS NOT NULL;
            """
        )

        conn.execute(
            """
            INSERT INTO people (wikidata_id, imdb_id, name)
            SELECT DISTINCT 'imdb:' || imdb_id, imdb_id, name FROM tmp_series_writers_named
            ON CONFLICT (imdb_id) DO UPDATE SET name = EXCLUDED.name;
            """
        )

        conn.execute(
            """
            INSERT INTO series_credits (series_id, person_id, job, department)
            SELECT DISTINCT w.series_id, p.id, 'Writer', 'Writing'
            FROM tmp_series_writers_named w
            JOIN people p ON w.imdb_id = p.imdb_id
            WHERE NOT EXISTS (
                SELECT 1 FROM series_credits sc 
                WHERE sc.series_id = w.series_id AND sc.person_id = p.id AND sc.job = 'Writer'
            );
            """
        )
        writer_count = conn.execute("SELECT COUNT(*) FROM series_credits WHERE job = 'Writer'").fetchone()[0]
        console.print(f"  -> {writer_count:,} roteiristas vinculados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 5. PONTES TRANS-MIDIÁTICAS E ADAPTAÇÕES LITERÁRIAS
        # ---------------------------------------------------------------------
        console.print("\n[bold green]5/6 Processando pontes de adaptações e realizadores...[/bold green]")
        t0 = time.time()

        # ---------------------------------------------------------------------
        # 6. ENRIQUECER 'graph_semantic_edges' COM AS ARESTAS DE SÉRIES
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]6/6 Integrando séries à malha ativa 'graph_semantic_edges'...[/bold magenta]")
        t0 = time.time()

        # Inserir Diretores de séries
        conn.execute(
            """
            INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
            SELECT DISTINCT
                'series:' || CAST(sc.series_id AS VARCHAR),
                'person:' || CAST(sc.person_id AS VARCHAR),
                2.0,
                'has_director'
            FROM series_credits sc
            WHERE sc.job = 'Director'
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges gse
                  WHERE gse.source_id = 'series:' || CAST(sc.series_id AS VARCHAR)
                    AND gse.target_id = 'person:' || CAST(sc.person_id AS VARCHAR)
                    AND gse.relation_type = 'has_director'
              );
            """
        )

        # Inserir Roteiristas de séries
        conn.execute(
            """
            INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
            SELECT DISTINCT
                'series:' || CAST(sc.series_id AS VARCHAR),
                'person:' || CAST(sc.person_id AS VARCHAR),
                1.8,
                'has_writer'
            FROM series_credits sc
            WHERE sc.job = 'Writer'
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges gse
                  WHERE gse.source_id = 'series:' || CAST(sc.series_id AS VARCHAR)
                    AND gse.target_id = 'person:' || CAST(sc.person_id AS VARCHAR)
                    AND gse.relation_type = 'has_writer'
              );
            """
        )

        # Inserir Gêneros / Keywords de séries
        conn.execute(
            """
            INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
            SELECT DISTINCT
                'series:' || CAST(sk.series_id AS VARCHAR),
                'keyword:' || CAST(sk.keyword_id AS VARCHAR),
                1.2,
                'has_tag'
            FROM series_keywords sk
            WHERE NOT EXISTS (
                SELECT 1 FROM graph_semantic_edges gse
                WHERE gse.source_id = 'series:' || CAST(sk.series_id AS VARCHAR)
                  AND gse.target_id = 'keyword:' || CAST(sk.keyword_id AS VARCHAR)
                  AND gse.relation_type = 'has_tag'
            );
            """
        )

        # Inserir Adaptações Literárias Diretas (Série ⇄ Livro)
        conn.execute(
            """
            INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
            SELECT DISTINCT
                'series:' || CAST(s.id AS VARCHAR),
                'book:' || CAST(b.id AS VARCHAR),
                3.0,
                'based_on'
            FROM tv_series s
            JOIN books b ON LOWER(TRIM(s.title)) = LOWER(TRIM(b.title))
            WHERE LENGTH(s.title) > 3
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges gse
                  WHERE gse.source_id = 'series:' || CAST(s.id AS VARCHAR)
                    AND gse.target_id = 'book:' || CAST(b.id AS VARCHAR)
                    AND gse.relation_type = 'based_on'
              );
            """
        )

        total_edges = conn.execute("SELECT COUNT(*) FROM graph_semantic_edges").fetchone()[0]
        total_series = conn.execute("SELECT COUNT(*) FROM tv_series").fetchone()[0]
        console.print(f"  -> Malha semântica enriquecida para {total_edges:,} arestas totais em {time.time() - t0:.2f}s!")

    elapsed_total = time.time() - start_total
    console.print(f"\n[bold green][ok] Ingestão de Séries concluída em {elapsed_total:.2f}s![/bold green]")

    # Exibir resumo em tabela
    table = Table(title="Estatísticas da Ingestão de Séries de TV", show_header=True, header_style="bold magenta")
    table.add_column("Métrica", style="dim")
    table.add_column("Valor", justify="right")
    table.add_row("Séries / Minisséries Ingeridas", f"{total_series:,}")
    table.add_row("Diretores / Showrunners Vinculados", f"{dir_count:,}")
    table.add_row("Roteiristas Vinculados", f"{writer_count:,}")
    table.add_row("Gêneros & Tags Conectados", f"{genre_edge_count:,}")
    table.add_row("Total de Arestas Ativas (graph_semantic_edges)", f"{total_edges:,}")
    table.add_row("Tempo de Execução", f"{elapsed_total:.2f}s")
    console.print(table)


if __name__ == "__main__":
    run_series_bulk_ingestion()
