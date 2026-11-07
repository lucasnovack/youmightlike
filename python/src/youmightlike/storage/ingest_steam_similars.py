from __future__ import annotations

import argparse
import time
from pathlib import Path

from rich.console import Console
from rich.table import Table

from youmightlike.storage.database import init_db

console = Console()

STEAM_INTERACTIONS_CSV = Path("data/raw/steam/steam-200k.csv")


def run_steam_similarity_ingestion(
    min_common_users: int = 5,
    min_score: float = 0.15,
    top_k_per_game: int = 15,
) -> None:
    """Calculates and ingests co-play similarity edges between Steam games into DuckDB."""
    if not STEAM_INTERACTIONS_CSV.exists():
        raise FileNotFoundError(
            f"Steam interactions file not found: {STEAM_INTERACTIONS_CSV}"
        )

    db = init_db()
    console.print("\n[bold cyan]Iniciando Cálculo e Ingestão de Similaridades da Steam (Co-jogabilidade)[/bold cyan]")
    console.print(
        f"⚙️ Filtros: Mínimo de [bold green]{min_common_users}[/bold green] jogadores compartilhados, "
        f"score [bold green]>={min_score}[/bold green], top-[bold green]{top_k_per_game}[/bold green] similares por jogo"
    )

    start_total = time.time()

    with db.connect() as conn:
        conn.execute("PRAGMA threads=8;")
        conn.execute("PRAGMA memory_limit='8GB';")

        # ---------------------------------------------------------------------
        # 0. CRIAR TABELA CANÔNICA game_similarities
        # ---------------------------------------------------------------------
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS game_similarities (
                source_game_id BIGINT NOT NULL REFERENCES games(id),
                target_game_id BIGINT NOT NULL REFERENCES games(id),
                match_score DOUBLE NOT NULL,
                PRIMARY KEY (source_game_id, target_game_id)
            );
            """
        )

        # ---------------------------------------------------------------------
        # 1. MAPEAMENTO DE NOMES DE JOGOS DA STEAM COM O BANCO
        # ---------------------------------------------------------------------
        console.print("\n[bold yellow]1/4 Mapeando títulos da Steam com o catálogo de games...[/bold yellow]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_matched_games AS
            WITH steam_games AS (
                SELECT DISTINCT column1 AS raw_title
                FROM read_csv('{STEAM_INTERACTIONS_CSV}', header=false)
            )
            SELECT 
                s.raw_title,
                g.id AS game_id,
                g.title
            FROM steam_games s
            JOIN games g ON 
                REGEXP_REPLACE(LOWER(TRIM(g.title)), '[^a-z0-9]', '', 'g') = 
                REGEXP_REPLACE(LOWER(TRIM(s.raw_title)), '[^a-z0-9]', '', 'g')
            QUALIFY ROW_NUMBER() OVER (
                PARTITION BY s.raw_title 
                ORDER BY 
                    CASE WHEN LOWER(TRIM(g.title)) = LOWER(TRIM(s.raw_title)) THEN 1 ELSE 0 END DESC,
                    COALESCE(g.positive_reviews, 0) DESC,
                    g.id ASC
            ) = 1;
            """
        )

        matched_count = conn.execute("SELECT COUNT(*) FROM tmp_matched_games").fetchone()[0]
        console.print(f"  -> {matched_count:,} jogos correlacionados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 2. AGREGAÇÃO DE INTERAÇÕES E CÁLCULO DE VETORES DE USUÁRIOS
        # ---------------------------------------------------------------------
        console.print("\n[bold magenta]2/4 Vetorizando interações de usuários (horas jogadas + compras)...[/bold magenta]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_user_game AS
            SELECT 
                raw.column0 AS user_id,
                m.game_id,
                ln(1.0 + MAX(CASE WHEN raw.column2 = 'play' THEN raw.column3 ELSE 0.5 END)) AS log_weight
            FROM read_csv('{STEAM_INTERACTIONS_CSV}', header=false) raw
            JOIN tmp_matched_games m ON raw.column1 = m.raw_title
            GROUP BY raw.column0, m.game_id;

            CREATE OR REPLACE TEMP TABLE tmp_game_norms AS
            SELECT 
                game_id, 
                SQRT(SUM(log_weight * log_weight)) AS norm
            FROM tmp_user_game
            GROUP BY game_id
            HAVING SUM(log_weight * log_weight) > 0;
            """
        )

        users_count = conn.execute("SELECT COUNT(DISTINCT user_id) FROM tmp_user_game").fetchone()[0]
        console.print(f"  -> {users_count:,} perfis de usuários vetorizados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 3. CÁLCULO DE SIMILARIDADE DE COSSENO (CO-PLAYING)
        # ---------------------------------------------------------------------
        console.print("\n[bold blue]3/4 Calculando similaridade de cosseno e top-K por jogo...[/bold blue]")
        t0 = time.time()

        conn.execute(
            f"""
            CREATE OR REPLACE TEMP TABLE tmp_raw_sims AS
            WITH pairs AS (
                SELECT 
                    u1.game_id AS source_id,
                    u2.game_id AS target_id,
                    SUM(u1.log_weight * u2.log_weight) AS dot_product,
                    COUNT(*) AS common_users
                FROM tmp_user_game u1
                JOIN tmp_user_game u2 ON u1.user_id = u2.user_id AND u1.game_id != u2.game_id
                GROUP BY u1.game_id, u2.game_id
                HAVING COUNT(*) >= {min_common_users}
            ),
            scored AS (
                SELECT 
                    p.source_id,
                    p.target_id,
                    ROUND((p.dot_product / (n1.norm * n2.norm)), 4) AS score,
                    ROW_NUMBER() OVER (
                        PARTITION BY p.source_id 
                        ORDER BY (p.dot_product / (n1.norm * n2.norm)) DESC
                    ) AS rn
                FROM pairs p
                JOIN tmp_game_norms n1 ON p.source_id = n1.game_id
                JOIN tmp_game_norms n2 ON p.target_id = n2.game_id
                WHERE (p.dot_product / (n1.norm * n2.norm)) >= {min_score}
            )
            SELECT source_id, target_id, score
            FROM scored
            WHERE rn <= {top_k_per_game};
            """
        )

        sim_pairs_count = conn.execute("SELECT COUNT(*) FROM tmp_raw_sims").fetchone()[0]
        console.print(f"  -> {sim_pairs_count:,} pares direcionados top-{top_k_per_game} calculados em {time.time() - t0:.2f}s!")

        # ---------------------------------------------------------------------
        # 4. INSERÇÃO BIDIRECIONAL EM game_similarities E graph_semantic_edges
        # ---------------------------------------------------------------------
        console.print("\n[bold green]4/4 Persistindo em game_similarities e graph_semantic_edges...[/bold green]")
        t0 = time.time()

        # Inserção simétrica em game_similarities
        conn.execute(
            """
            INSERT INTO game_similarities (source_game_id, target_game_id, match_score)
            SELECT source_id, target_id, score FROM tmp_raw_sims
            ON CONFLICT (source_game_id, target_game_id) DO UPDATE SET
                match_score = GREATEST(game_similarities.match_score, EXCLUDED.match_score);

            INSERT INTO game_similarities (source_game_id, target_game_id, match_score)
            SELECT target_id, source_id, score FROM tmp_raw_sims
            ON CONFLICT (source_game_id, target_game_id) DO UPDATE SET
                match_score = GREATEST(game_similarities.match_score, EXCLUDED.match_score);
            """
        )

        total_sims = conn.execute("SELECT COUNT(*) FROM game_similarities").fetchone()[0]
        console.print(f"  -> Tabela 'game_similarities' atualizada: {total_sims:,} arestas.")

        # Inserir em graph_semantic_edges
        # Remove eventuais arestas anteriores game-game similar_to para idempotência
        conn.execute(
            """
            DELETE FROM graph_semantic_edges
            WHERE source_id LIKE 'game:%'
              AND target_id LIKE 'game:%'
              AND relation_type = 'similar_to';
            """
        )

        conn.execute(
            """
            INSERT INTO graph_semantic_edges (source_id, target_id, base_weight, relation_type)
            SELECT 
                'game:' || CAST(source_game_id AS VARCHAR),
                'game:' || CAST(target_game_id AS VARCHAR),
                ROUND(match_score * 2.0, 4) AS base_weight,
                'similar_to' AS relation_type
            FROM game_similarities;
            """
        )

        inserted_edges = conn.execute(
            """
            SELECT COUNT(*) FROM graph_semantic_edges
            WHERE source_id LIKE 'game:%' AND relation_type = 'similar_to'
            """
        ).fetchone()[0]

        console.print(f"  -> {inserted_edges:,} arestas 'similar_to' inseridas em graph_semantic_edges em {time.time() - t0:.2f}s!")

        total_time = time.time() - start_total
        console.print(f"\n[bold green]Ingestão de co-jogabilidade finalizada com sucesso em {total_time:.2f}s![/bold green]\n")

        # Exibir Amostra
        sample_table = Table(title="Amostra de Jogos Similares Ingeridos", show_header=True, header_style="bold cyan")
        sample_table.add_column("Jogo Origem", style="bold")
        sample_table.add_column("Jogo Similar", style="bold green")
        sample_table.add_column("Score Cosine", justify="right")
        sample_table.add_column("Peso Base (Graph)", justify="right")

        sample_rows = conn.execute(
            """
            SELECT g1.title, g2.title, s.match_score, ROUND(s.match_score * 2.0, 4)
            FROM game_similarities s
            JOIN games g1 ON s.source_game_id = g1.id
            JOIN games g2 ON s.target_game_id = g2.id
            WHERE g1.title ILIKE '%skyrim%' OR g1.title ILIKE '%portal 2%'
            ORDER BY s.match_score DESC
            LIMIT 10;
            """
        ).fetchall()

        for r in sample_rows:
            sample_table.add_row(r[0], r[1], f"{r[2]:.4f}", f"{r[3]:.4f}")

        console.print(sample_table)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Ingest Steam co-play similarities into DuckDB")
    parser.add_argument("--min-users", type=int, default=5, help="Minimum common users")
    parser.add_argument("--min-score", type=float, default=0.15, help="Minimum cosine similarity")
    parser.add_argument("--top-k", type=int, default=15, help="Top K similar games per game")
    args = parser.parse_args()

    run_steam_similarity_ingestion(
        min_common_users=args.min_users,
        min_score=args.min_score,
        top_k_per_game=args.top_k,
    )
