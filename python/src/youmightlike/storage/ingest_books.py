from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

PARQUET_FILE = Path("data/raw/books/goodreads_100k.parquet")


def run_books_bulk_ingestion(min_ratings: int = 300) -> None:
    """Ingere livros, autores e temas/gêneros diretamente do Parquet no DuckDB."""
    if not PARQUET_FILE.exists():
        raise FileNotFoundError(f"Arquivo Parquet não encontrado: {PARQUET_FILE}")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Ingestão em Massa de Livros via DuckDB (Parquet Nativo)[/bold cyan]")
    console.print(f"️ Critério de corte: Livros com [bold green]>={min_ratings}[/bold green] avaliações")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. FILTRAR LIVROS QUALIFICADOS
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Filtrando e inserindo livros qualificados...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_filtered_books AS
            SELECT 
                'gr:' || REGEXP_EXTRACT(link, 'show/([0-9]+)', 1) AS work_key,
                TRIM(title) AS title,
                TRIM(author) AS authors_str,
                TRIM(COALESCE("desc", '')) AS description,
                genre,
                totalratings,
                rating
            FROM '{PARQUET_FILE}'
            WHERE totalratings >= {min_ratings}
              AND title IS NOT NULL AND TRIM(title) != ''
              AND author IS NOT NULL AND TRIM(author) != ''
              AND REGEXP_EXTRACT(link, 'show/([0-9]+)', 1) != '';
            """
        )

        book_count = conn.execute("SELECT COUNT(*) FROM tmp_filtered_books").fetchone()[0]
        console.print(f"  -> {book_count:,} livros qualificados selecionados em {time.time() - t0:.2f}s!")

        # Inserir na tabela canônica 'books'
        # Evitamos sobrescrever o que já existe se já tiver work_key
        conn.execute(
            """
            INSERT INTO books (work_key, title, description)
            SELECT work_key, title, description
            FROM tmp_filtered_books
            ON CONFLICT (work_key) DO UPDATE SET
                title = EXCLUDED.title,
                description = CASE WHEN books.description IS NULL OR books.description = '' THEN EXCLUDED.description ELSE books.description END;
            """
        )
        console.print(f"  -> Tabela 'books' atualizada.")

        # ---------------------------------------------------------------------
        # 2. AUTORES DE LIVROS (book_authors)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]️ 2/4 Mapeando autores de livros...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_book_authors_flat AS
            SELECT 
                b.id AS book_id,
                TRIM(UNNEST(STRING_SPLIT(tmp.authors_str, ','))) AS author_name
            FROM tmp_filtered_books tmp
            JOIN books b ON b.work_key = tmp.work_key
            WHERE tmp.authors_str IS NOT NULL;
            """
        )

        # Inserir autores únicos em book_authors
        conn.execute(
            """
            INSERT INTO book_authors (author_key, name)
            SELECT DISTINCT 
                'author:' || md5(lower(author_name)) AS author_key,
                author_name AS name
            FROM tmp_book_authors_flat
            WHERE author_name != '' AND length(author_name) > 1
            ON CONFLICT (author_key) DO NOTHING;
            """
        )

        author_count = conn.execute("SELECT COUNT(*) FROM book_authors").fetchone()[0]
        console.print(f"  -> Total de autores no banco: {author_count:,} em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. RELAÇÃO LIVRO -> AUTOR (book_author_links)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]3/4 Conectando livros aos seus autores (written_by)...[/bold blue]")
        t0 = time.time()

        conn.execute(
            """
            INSERT INTO book_author_links (book_id, author_id)
            SELECT DISTINCT
                f.book_id,
                ba.id AS author_id
            FROM tmp_book_authors_flat f
            JOIN book_authors ba ON ba.author_key = ('author:' || md5(lower(f.author_name)))
            WHERE f.author_name != ''
            ON CONFLICT (book_id, author_id) DO NOTHING;
            """
        )

        links_count = conn.execute("SELECT COUNT(*) FROM book_author_links").fetchone()[0]
        console.print(f"  -> {links_count:,} arestas de autoria registradas em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 4. GÊNEROS E TEMAS (keywords & book_subjects)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]️ 4/4 Mapeando temas e tópicos dos livros (has_subject)...[/bold green]")
        t0 = time.time()

        conn.execute(
            """
            CREATE OR REPLACE TEMP TABLE tmp_book_tags_flat AS
            SELECT 
                b.id AS book_id,
                LOWER(TRIM(UNNEST(STRING_SPLIT(tmp.genre, ',')))) AS tag_name
            FROM tmp_filtered_books tmp
            JOIN books b ON b.work_key = tmp.work_key
            WHERE tmp.genre IS NOT NULL AND TRIM(tmp.genre) != '';
            """
        )

        # Inserir novos keywords no vocabulário unificado
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'goodreads'
            FROM tmp_book_tags_flat
            WHERE length(tag_name) >= 2
            ON CONFLICT (name) DO NOTHING;
            """
        )

        # Vincular livros aos keywords (book_subjects)
        conn.execute(
            """
            INSERT INTO book_subjects (book_id, keyword_id)
            SELECT DISTINCT
                f.book_id,
                k.id AS keyword_id
            FROM tmp_book_tags_flat f
            JOIN keywords k ON k.name = f.tag_name
            WHERE length(f.tag_name) >= 2
            ON CONFLICT (book_id, keyword_id) DO NOTHING;
            """
        )

        subjects_count = conn.execute("SELECT COUNT(*) FROM book_subjects").fetchone()[0]
        total_keywords = conn.execute("SELECT COUNT(*) FROM keywords").fetchone()[0]
        console.print(f"  -> {subjects_count:,} conexões temáticas criadas ({total_keywords:,} keywords globais) em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de Livros concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

        # Exibir censo consolidado
        _print_summary(conn)


def _print_summary(conn) -> None:
    table = Table(title="Censo Geral do Grafo de Conhecimento (DuckDB)", show_header=True, header_style="bold cyan")
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
        ("Keywords Globais (keywords)", "SELECT count(*) FROM keywords"),
        ("Nós no Grafo (v_graph_nodes)", "SELECT count(*) FROM v_graph_nodes"),
        ("Arestas Ativas (v_graph_edges)", "SELECT count(*) FROM v_graph_edges"),
    ]

    for name, query in tables:
        count = conn.execute(query).fetchone()[0]
        table.add_row(name, f"{count:,}")

    console.print(table)


if __name__ == "__main__":
    run_books_bulk_ingestion()
