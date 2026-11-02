from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

TAGS_CSV = Path("data/raw/movielens/genome_tags.csv")
LINKS_CSV = Path("data/raw/movielens/link.csv")
SCORES_CSV = Path("data/raw/movielens/genome_scores.csv")


def run_movielens_bulk_ingestion(min_relevance: float = 0.65) -> None:
    """Ingere tags temáticas e semânticas do MovieLens Tag Genome conectando aos filmes existentes."""
    if not TAGS_CSV.exists() or not LINKS_CSV.exists() or not SCORES_CSV.exists():
        raise FileNotFoundError("Arquivos do MovieLens Tag Genome não encontrados em data/raw/movielens/")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Ingestão do MovieLens Tag Genome (Semântica de Filmes)[/bold cyan]")
    console.print(f"️ Critério de corte: Tags com relevância [bold green]>={min_relevance}[/bold green] (escala 0.0 a 1.0)")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. MAPEAMENTO DE FILMES EXISTENTES VIA IMDB_ID
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/3 Vinculando links do MovieLens aos filmes existentes no banco...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_matched_movies AS
            SELECT 
                m.id AS movie_id,
                l.movieId
            FROM movies m
            JOIN read_csv_auto('{LINKS_CSV}') l 
              ON m.imdb_id = ('tt' || LPAD(CAST(l.imdbId AS VARCHAR), 7, '0'))
            WHERE l.imdbId IS NOT NULL;
            """
        )

        matched_count = conn.execute("SELECT COUNT(*) FROM tmp_matched_movies").fetchone()[0]
        console.print(f"  -> {matched_count:,} filmes mapeados com sucesso em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 2. SELEÇÃO DE TAGS RELEVANTES E INSERÇÃO EM KEYWORDS
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]️ 2/3 Filtrando tags de alta relevância e enriquecendo vocabulário...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_movie_tags_flat AS
            SELECT 
                mm.movie_id,
                LOWER(TRIM(gt.tag)) AS tag_name,
                s.relevance
            FROM read_csv_auto('{SCORES_CSV}') s
            JOIN tmp_matched_movies mm ON s.movieId = mm.movieId
            JOIN read_csv_auto('{TAGS_CSV}') gt ON s.tagId = gt.tagId
            WHERE s.relevance >= {min_relevance}
              AND gt.tag IS NOT NULL AND LENGTH(TRIM(gt.tag)) > 1;
            """
        )

        # Inserir novas tags na tabela canônica 'keywords' (sem duplicar as existentes)
        conn.execute(
            """
            INSERT INTO keywords (name, source)
            SELECT DISTINCT tag_name, 'movielens'
            FROM tmp_movie_tags_flat
            ON CONFLICT (name) DO NOTHING;
            """
        )

        total_keywords = conn.execute("SELECT COUNT(*) FROM keywords").fetchone()[0]
        console.print(f"  -> Vocabulário global atualizado para {total_keywords:,} keywords em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. VINCULAR FILMES ÀS KEYWORDS (movie_keywords / has_keyword)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]3/3 Criando arestas conceituais Filme Keyword (has_keyword)...[/bold green]")
        t0 = time.time()

        conn.execute(
            """
            INSERT INTO movie_keywords (movie_id, keyword_id)
            SELECT DISTINCT
                f.movie_id,
                k.id AS keyword_id
            FROM tmp_movie_tags_flat f
            JOIN keywords k ON k.name = f.tag_name
            ON CONFLICT (movie_id, keyword_id) DO NOTHING;
            """
        )

        movie_kw_count = conn.execute("SELECT COUNT(*) FROM movie_keywords").fetchone()[0]
        console.print(f"  -> {movie_kw_count:,} arestas de keywords em filmes registradas em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão do Tag Genome concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

        _print_summary(conn)


def _print_summary(conn) -> None:
    table = Table(title="Censo Geral Consolidado do Grafo Transmidiático (DuckDB)", show_header=True, header_style="bold cyan")
    table.add_column("Domínio / Tabela", style="bold")
    table.add_column("Total de Registros", justify="right", style="green")

    tables = [
        ("Filmes (movies)", "SELECT count(*) FROM movies"),
        ("Créditos de Filmes (movie_credits)", "SELECT count(*) FROM movie_credits"),
        ("Keywords de Filmes (movie_keywords)", "SELECT count(*) FROM movie_keywords"),
        ("Jogos (games)", "SELECT count(*) FROM games"),
        ("Tags de Jogos (game_tags)", "SELECT count(*) FROM game_tags"),
        ("Livros (books)", "SELECT count(*) FROM books"),
        ("Temas de Livros (book_subjects)", "SELECT count(*) FROM book_subjects"),
        ("Artistas Musicais (music_artists)", "SELECT count(*) FROM music_artists"),
        ("Tags de Música (artist_tags)", "SELECT count(*) FROM artist_tags"),
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
    run_movielens_bulk_ingestion()
