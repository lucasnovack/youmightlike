from __future__ import annotations

import time
from pathlib import Path
from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

ARTISTS_DAT = Path("data/raw/lastfm/artists.dat")
USER_ARTISTS_DAT = Path("data/raw/lastfm/user_artists.dat")


def run_lastfm_similarity_ingestion(min_common_listeners: int = 3, min_score: float = 0.15) -> None:
    """Calcula e ingere arestas de similaridade direta entre artistas a partir do grafo de co-escuta do Last.fm."""
    if not ARTISTS_DAT.exists() or not USER_ARTISTS_DAT.exists():
        raise FileNotFoundError("Arquivos do Last.fm (artists.dat, user_artists.dat) não encontrados em data/raw/lastfm/")

    db = init_db()
    console.print("\n[bold cyan]Iniciando Cálculo e Ingestão de Similaridades do Last.fm[/bold cyan]")
    console.print(f"️ Filtros: Mínimo de [bold green]{min_common_listeners}[/bold green] ouvintes compartilhados e score [bold green]>={min_score}[/bold green]")

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 1. MAPEAMENTO DE ARTISTAS COM O BANCO EXISTENTE
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/3 Mapeando artistas do Last.fm com o catálogo canônico...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_matched_artists AS
            WITH la AS (
                SELECT id AS lastfm_id, TRIM(name) AS name 
                FROM read_csv('{ARTISTS_DAT}', delim='\t', header=true)
            )
            SELECT la.lastfm_id, ma.id AS db_artist_id, ma.name
            FROM la
            JOIN music_artists ma ON lower(trim(ma.name)) = lower(trim(la.name));
            """
        )

        matched_count = conn.execute("SELECT COUNT(DISTINCT db_artist_id) FROM tmp_matched_artists").fetchone()[0]
        console.print(f"  -> {matched_count:,} artistas únicos correlacionados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 2. CÁLCULO DE SIMILARIDADE DE CO-ESCUTA (COSINE COM LOG-WEIGHT)
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]2/3 Calculando similaridade de cosseno de co-ocorrência em CPU...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_artist_sims AS
            WITH user_art AS (
                SELECT 
                    ua.userID, 
                    ma.db_artist_id, 
                    ma.name,
                    ln(1 + ua.weight) AS log_weight
                FROM read_csv('{USER_ARTISTS_DAT}', delim='\t', header=true) ua
                JOIN tmp_matched_artists ma ON ua.artistID = ma.lastfm_id
            ),
            norms AS (
                SELECT db_artist_id, sqrt(sum(log_weight * log_weight)) AS norm
                FROM user_art
                GROUP BY db_artist_id
                HAVING sum(log_weight * log_weight) > 0
            ),
            pairs AS (
                SELECT 
                    u1.db_artist_id AS source_id,
                    u2.db_artist_id AS target_id,
                    sum(u1.log_weight * u2.log_weight) AS dot_product,
                    count(*) AS common_listeners
                FROM user_art u1
                JOIN user_art u2 ON u1.userID = u2.userID AND u1.db_artist_id < u2.db_artist_id
                WHERE lower(trim(u1.name)) != lower(trim(u2.name))
                GROUP BY u1.db_artist_id, u2.db_artist_id
                HAVING count(*) >= {min_common_listeners}
            )
            SELECT 
                p.source_id,
                p.target_id,
                ROUND((p.dot_product / (n1.norm * n2.norm)), 4) AS score
            FROM pairs p
            JOIN norms n1 ON p.source_id = n1.db_artist_id
            JOIN norms n2 ON p.target_id = n2.db_artist_id
            WHERE (p.dot_product / (n1.norm * n2.norm)) >= {min_score};
            """
        )

        sim_pairs_count = conn.execute("SELECT COUNT(*) FROM tmp_artist_sims").fetchone()[0]
        console.print(f"  -> {sim_pairs_count:,} pares de alta afinidade calculados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. INSERÇÃO BIDIRECIONAL EM artist_similarities (similar_to)
        # ---------------------------------------------------------------------
        console.print("\n[bold green]3/3 Inserindo arestas bidirecionais 'similar_to' no DuckDB...[/bold green]")
        t0 = time.time()

        # Inserção A -> B
        conn.execute(
            """
            INSERT INTO artist_similarities (source_artist_id, target_artist_id, match_score)
            SELECT source_id, target_id, score
            FROM tmp_artist_sims
            ON CONFLICT (source_artist_id, target_artist_id) DO UPDATE SET
                match_score = GREATEST(artist_similarities.match_score, EXCLUDED.match_score);
            """
        )

        # Inserção B -> A (grafo de similaridade simétrico)
        conn.execute(
            """
            INSERT INTO artist_similarities (source_artist_id, target_artist_id, match_score)
            SELECT target_id, source_id, score
            FROM tmp_artist_sims
            ON CONFLICT (source_artist_id, target_artist_id) DO UPDATE SET
                match_score = GREATEST(artist_similarities.match_score, EXCLUDED.match_score);
            """
        )

        total_sims = conn.execute("SELECT COUNT(*) FROM artist_similarities").fetchone()[0]
        console.print(f"  -> Tabela 'artist_similarities' populada com {total_sims:,} arestas em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de Similaridades concluída com sucesso em {total_time:.2f} segundos![/bold green]\n")

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
    run_lastfm_similarity_ingestion()
