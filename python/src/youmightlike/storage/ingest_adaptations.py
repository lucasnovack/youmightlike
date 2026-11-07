"""
Ingestion pipeline for literary adaptations (Book <-> Movie, Book <-> TV Series).
Populates based_on and adapted_as direct transmedia bridges in taste_graph.duckdb.
"""

from __future__ import annotations

import time
from pathlib import Path

import duckdb
from rich.console import Console
from rich.table import Table

console = Console()

DB_PATH = Path("data/processed/taste_graph.duckdb")
CSV_PATH = Path("data/raw/wikidata/adaptations.csv")


def run_adaptations_ingestion(db_path: Path = DB_PATH, csv_path: Path = CSV_PATH) -> None:
    if not csv_path.exists():
        raise FileNotFoundError(f"Adaptations CSV not found at {csv_path}")

    console.print("\n[bold cyan]Iniciando Ingestão em Massa de Adaptações Literárias (Wikidata ➔ DuckDB)[/bold cyan]")
    start_time = time.time()

    con = duckdb.connect(str(db_path))
    con.execute("PRAGMA threads=8;")
    con.execute("PRAGMA memory_limit='8GB';")

    try:
        # 1. Carregar CSV bruto em tabela temporária
        console.print("[bold yellow]1/5 Carregando e limpando dados brutos da Wikidata...[/bold yellow]")
        con.execute(f"""
            CREATE OR REPLACE TEMP TABLE tmp_raw_adaptations AS
            SELECT 
                TRIM(imdb_id) AS imdb_id,
                REGEXP_EXTRACT(work, 'entity/(Q[0-9]+)', 1) AS work_qid,
                TRIM(workLabel) AS work_title,
                TRIM(authorLabel) AS author_name,
                TRY_CAST(pubYear AS INTEGER) AS pub_year
            FROM read_csv_auto('{csv_path}')
            WHERE imdb_id IS NOT NULL 
              AND work IS NOT NULL 
              AND TRIM(workLabel) != ''
              AND REGEXP_EXTRACT(work, 'entity/(Q[0-9]+)', 1) != ''
        """)

        # Filtrar apenas adaptações cujas obras audiovisuais estão no nosso catálogo
        con.execute("""
            CREATE OR REPLACE TEMP TABLE tmp_catalog_adaptations AS
            SELECT DISTINCT 
                r.imdb_id,
                r.work_qid,
                r.work_title,
                NULLIF(r.author_name, '') AS author_name,
                r.pub_year,
                m.id AS movie_id,
                s.id AS series_id
            FROM tmp_raw_adaptations r
            LEFT JOIN movies m ON m.imdb_id = r.imdb_id
            LEFT JOIN tv_series s ON s.imdb_id = r.imdb_id
            WHERE m.id IS NOT NULL OR s.id IS NOT NULL
        """)

        total_cat_adaptations = con.execute("SELECT COUNT(*) FROM tmp_catalog_adaptations").fetchone()[0]
        distinct_works = con.execute("SELECT COUNT(DISTINCT work_qid) FROM tmp_catalog_adaptations").fetchone()[0]
        console.print(f"  -> {total_cat_adaptations:,} adaptações vinculadas a filmes/séries ({distinct_works:,} obras literárias únicas).")

        # 2. Casar ou criar livros na tabela 'books'
        console.print("\n[bold yellow]2/5 Mapeando e integrando livros na tabela 'books'...[/bold yellow]")
        con.execute("""
            CREATE OR REPLACE TEMP TABLE tmp_distinct_works AS
            SELECT 
                work_qid,
                FIRST(work_title) AS work_title,
                FIRST(author_name) AS author_name,
                MIN(pub_year) AS first_publish_year
            FROM tmp_catalog_adaptations
            GROUP BY work_qid
        """)

        con.execute("""
            CREATE OR REPLACE TEMP TABLE tmp_new_books AS
            SELECT w.work_qid, w.work_title, w.first_publish_year
            FROM tmp_distinct_works w
            WHERE w.work_qid NOT IN (SELECT wikidata_id FROM books WHERE wikidata_id IS NOT NULL)
              AND LOWER(TRIM(w.work_title)) NOT IN (SELECT LOWER(TRIM(title)) FROM books WHERE title IS NOT NULL)
        """)

        new_books_count = con.execute("SELECT COUNT(*) FROM tmp_new_books").fetchone()[0]
        con.execute("""
            INSERT INTO books (title, wikidata_id, first_publish_year)
            SELECT work_title, work_qid, first_publish_year
            FROM tmp_new_books
        """)
        console.print(f"  -> {new_books_count:,} novos livros inseridos na tabela canônica 'books'.")

        # Mapeamento work_qid -> book_id
        con.execute("""
            CREATE OR REPLACE TEMP TABLE tmp_work_book_map AS
            SELECT 
                w.work_qid,
                COALESCE(b_qid.id, b_title.id) AS book_id
            FROM tmp_distinct_works w
            LEFT JOIN books b_qid ON b_qid.wikidata_id = w.work_qid
            LEFT JOIN books b_title ON b_title.wikidata_id IS NULL AND LOWER(TRIM(b_title.title)) = LOWER(TRIM(w.work_title))
            QUALIFY ROW_NUMBER() OVER (PARTITION BY w.work_qid ORDER BY (CASE WHEN b_qid.id IS NOT NULL THEN 0 ELSE 1 END)) = 1
        """)
        mapped_count = con.execute("SELECT COUNT(*), COUNT(book_id) FROM tmp_work_book_map").fetchone()
        console.print(f"  -> {mapped_count[1]:,} de {mapped_count[0]:,} obras mapeadas com ID canônico de 'books'.")

        # 3. Mapear e vincular autores
        console.print("\n[bold yellow]3/5 Vinculando autores e links literários (written_by)...[/bold yellow]")
        con.execute("""
            CREATE OR REPLACE TEMP TABLE tmp_authors_to_link AS
            SELECT DISTINCT 
                m.book_id,
                a.author_name
            FROM tmp_catalog_adaptations a
            JOIN tmp_work_book_map m ON m.work_qid = a.work_qid
            WHERE a.author_name IS NOT NULL AND a.author_name != '' AND m.book_id IS NOT NULL
        """)

        # Inserir autores faltantes
        con.execute("""
            INSERT INTO book_authors (author_key, name)
            SELECT DISTINCT
                'author:' || md5(lower(author_name)) AS author_key,
                author_name AS name
            FROM tmp_authors_to_link
            WHERE ('author:' || md5(lower(author_name))) NOT IN (SELECT author_key FROM book_authors)
        """)

        # Inserir links book_author_links
        con.execute("""
            INSERT INTO book_author_links (book_id, author_id)
            SELECT DISTINCT
                t.book_id,
                ba.id AS author_id
            FROM tmp_authors_to_link t
            JOIN book_authors ba ON ba.author_key = 'author:' || md5(lower(t.author_name))
            ON CONFLICT DO NOTHING
        """)

        # Adicionar arestas book -> author no graph_semantic_edges
        new_author_edges = con.execute("""
            INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
            SELECT DISTINCT
                'book:' || CAST(t.book_id AS VARCHAR) AS source_id,
                'author:' || CAST(ba.id AS VARCHAR) AS target_id,
                'written_by' AS relation_type,
                2.0 AS base_weight
            FROM tmp_authors_to_link t
            JOIN book_authors ba ON ba.author_key = 'author:' || md5(lower(t.author_name))
            WHERE NOT EXISTS (
                SELECT 1 FROM graph_semantic_edges e
                WHERE e.source_id = 'book:' || CAST(t.book_id AS VARCHAR)
                  AND e.target_id = 'author:' || CAST(ba.id AS VARCHAR)
                  AND e.relation_type = 'written_by'
            )
        """).fetchone()[0]
        console.print(f"  -> {new_author_edges:,} arestas 'written_by' adicionadas ao grafo.")

        # 4. Atualizar colunas based_on em movies e tv_series
        console.print("\n[bold yellow]4/5 Atualizando metadados de adaptação em movies e tv_series...[/bold yellow]")
        con.execute("""
            UPDATE movies
            SET based_on_title = a.work_title,
                based_on_wikidata_id = a.work_qid
            FROM (
                SELECT imdb_id, work_title, work_qid
                FROM tmp_catalog_adaptations
                WHERE movie_id IS NOT NULL
                QUALIFY ROW_NUMBER() OVER (PARTITION BY imdb_id ORDER BY work_qid) = 1
            ) a
            WHERE movies.imdb_id = a.imdb_id
        """)

        con.execute("""
            UPDATE tv_series
            SET based_on_title = a.work_title,
                based_on_wikidata_id = a.work_qid
            FROM (
                SELECT imdb_id, work_title, work_qid
                FROM tmp_catalog_adaptations
                WHERE series_id IS NOT NULL
                QUALIFY ROW_NUMBER() OVER (PARTITION BY imdb_id ORDER BY work_qid) = 1
            ) a
            WHERE tv_series.imdb_id = a.imdb_id
        """)

        # 5. Inserir arestas semânticas bidirecionais (based_on & adapted_as)
        console.print("\n[bold yellow]5/5 Injetando pontes transmidiáticas 'based_on' e 'adapted_as'...[/bold yellow]")
        # 5a. Filmes -> Livros (based_on) e Livros -> Filmes (adapted_as)
        new_movie_adapt = con.execute("""
            INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
            SELECT DISTINCT
                'movie:' || CAST(a.movie_id AS VARCHAR) AS source_id,
                'book:' || CAST(m.book_id AS VARCHAR) AS target_id,
                'based_on' AS relation_type,
                3.0 AS base_weight
            FROM tmp_catalog_adaptations a
            JOIN tmp_work_book_map m ON m.work_qid = a.work_qid
            WHERE a.movie_id IS NOT NULL AND m.book_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges e
                  WHERE e.source_id = 'movie:' || CAST(a.movie_id AS VARCHAR)
                    AND e.target_id = 'book:' || CAST(m.book_id AS VARCHAR)
                    AND e.relation_type = 'based_on'
              )
        """).fetchone()[0]

        new_book_movie = con.execute("""
            INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
            SELECT DISTINCT
                'book:' || CAST(m.book_id AS VARCHAR) AS source_id,
                'movie:' || CAST(a.movie_id AS VARCHAR) AS target_id,
                'adapted_as' AS relation_type,
                3.0 AS base_weight
            FROM tmp_catalog_adaptations a
            JOIN tmp_work_book_map m ON m.work_qid = a.work_qid
            WHERE a.movie_id IS NOT NULL AND m.book_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges e
                  WHERE e.source_id = 'book:' || CAST(m.book_id AS VARCHAR)
                    AND e.target_id = 'movie:' || CAST(a.movie_id AS VARCHAR)
                    AND e.relation_type = 'adapted_as'
              )
        """).fetchone()[0]

        # 5b. Séries -> Livros (based_on) e Livros -> Séries (adapted_as)
        new_series_adapt = con.execute("""
            INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
            SELECT DISTINCT
                'series:' || CAST(a.series_id AS VARCHAR) AS source_id,
                'book:' || CAST(m.book_id AS VARCHAR) AS target_id,
                'based_on' AS relation_type,
                3.0 AS base_weight
            FROM tmp_catalog_adaptations a
            JOIN tmp_work_book_map m ON m.work_qid = a.work_qid
            WHERE a.series_id IS NOT NULL AND m.book_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges e
                  WHERE e.source_id = 'series:' || CAST(a.series_id AS VARCHAR)
                    AND e.target_id = 'book:' || CAST(m.book_id AS VARCHAR)
                    AND e.relation_type = 'based_on'
              )
        """).fetchone()[0]

        new_book_series = con.execute("""
            INSERT INTO graph_semantic_edges (source_id, target_id, relation_type, base_weight)
            SELECT DISTINCT
                'book:' || CAST(m.book_id AS VARCHAR) AS source_id,
                'series:' || CAST(a.series_id AS VARCHAR) AS target_id,
                'adapted_as' AS relation_type,
                3.0 AS base_weight
            FROM tmp_catalog_adaptations a
            JOIN tmp_work_book_map m ON m.work_qid = a.work_qid
            WHERE a.series_id IS NOT NULL AND m.book_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM graph_semantic_edges e
                  WHERE e.source_id = 'book:' || CAST(m.book_id AS VARCHAR)
                    AND e.target_id = 'series:' || CAST(a.series_id AS VARCHAR)
                    AND e.relation_type = 'adapted_as'
              )
        """).fetchone()[0]

        total_new_edges = new_movie_adapt + new_book_movie + new_series_adapt + new_book_series
        console.print(f"  -> {new_movie_adapt:,} arestas movie ➔ book ('based_on')")
        console.print(f"  -> {new_book_movie:,} arestas book ➔ movie ('adapted_as')")
        console.print(f"  -> {new_series_adapt:,} arestas series ➔ book ('based_on')")
        console.print(f"  -> {new_book_series:,} arestas book ➔ series ('adapted_as')")
        console.print(f"  -> Total de novas pontes de adaptação: [bold green]{total_new_edges:,}[/bold green]")

        elapsed = time.time() - start_time
        console.print(f"\n[bold green]✓ Ingestão concluída com sucesso em {elapsed:.2f}s![/bold green]")

        # Tabela resumo final
        table = Table(title="Estatísticas das Pontes de Adaptação Transmidiática")
        table.add_column("Métrica", style="cyan")
        table.add_column("Quantidade", justify="right", style="green")

        total_edges = con.execute("SELECT COUNT(*) FROM graph_semantic_edges").fetchone()[0]
        total_books = con.execute("SELECT COUNT(*) FROM books").fetchone()[0]
        total_based_on = con.execute("SELECT COUNT(*) FROM graph_semantic_edges WHERE relation_type IN ('based_on', 'adapted_as')").fetchone()[0]

        table.add_row("Total de Livros no Catálogo", f"{total_books:,}")
        table.add_row("Pontes Ativas de Adaptação (based_on + adapted_as)", f"{total_based_on:,}")
        table.add_row("Total de Arestas Semânticas (graph_semantic_edges)", f"{total_edges:,}")
        console.print(table)

    finally:
        con.close()


if __name__ == "__main__":
    run_adaptations_ingestion()
