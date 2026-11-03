from __future__ import annotations

import sys
from typing import List
from rich.console import Console
from rich.panel import Panel
from rich.table import Table
from rich.prompt import Prompt

from youmightlike.resolution.resolver import MusicResolver, ResolvedEntity
from youmightlike.recommendation.engine import MusicRecommenderEngine

console = Console()


def render_banner() -> None:
    console.print(
        Panel(
            "[bold white]youmightlike[/bold white] — [dim]Deterministic Cultural Graph Discovery[/dim]\n"
            "[dim]40.4M Tracks • 3.2M Artists • 18M Albums • Zero LLMs • Local DuckDB + Rust Core[/dim]",
            border_style="dim",
            title="youmightlike",
            subtitle="v0.3.0",
        )
    )


def interactive_seed_selection(resolver: MusicResolver) -> List[ResolvedEntity]:
    seeds: List[ResolvedEntity] = []
    console.print("\n[bold]Select 1 to 4 tracks or artists as diffusion seeds.[/bold]")
    console.print("[dim](Press Enter on an empty line when finished)[/dim]\n")

    while len(seeds) < 4:
        count_label = f"[{len(seeds) + 1}/4]"
        query = Prompt.ask(f"[bold]Search entity[/bold] {count_label} (or Enter to finish)")
        query = query.strip()
        if not query:
            if seeds:
                break
            console.print("[red]Please add at least one seed node.[/red]")
            continue

        console.print(f"[dim]Querying catalog for '{query}'...[/dim]")
        candidates = resolver.resolve(query, limit=5)

        if not candidates:
            console.print(f"[yellow]No entities found for '{query}'. Try another query.[/yellow]\n")
            continue

        # Exibe os candidatos
        console.print(f"\n[bold]Found {len(candidates)} matches:[/bold]")
        for idx, c in enumerate(candidates, 1):
            console.print(f"  [bold white]{idx}[/bold white]. {c.display_label()}")

        choice_str = Prompt.ask(
            "Select number (or 0 to discard)",
            choices=[str(i) for i in range(len(candidates) + 1)],
            default="1",
        )
        choice = int(choice_str)
        if choice > 0:
            selected = candidates[choice - 1]
            seeds.append(selected)
            console.print(f"[green][ok][/green] Added seed: {selected.display_label()}\n")
        else:
            console.print("[dim]Discarded.[/dim]\n")

    return seeds


def run_cli() -> None:
    render_banner()

    resolver = MusicResolver()
    engine = MusicRecommenderEngine()

    seeds: List[ResolvedEntity] = []

    # Se o usuário passou argumentos na linha de comando, resolve direto
    if len(sys.argv) > 1:
        raw_queries = sys.argv[1:]
        console.print(f"\n[bold]Resolving CLI seed arguments: {raw_queries}[/bold]")
        for q in raw_queries:
            found = resolver.resolve(q, limit=1)
            if found:
                seeds.append(found[0])
                console.print(f"  [green][ok][/green] Resolved: {found[0].display_label()}")
            else:
                console.print(f"  [red][error][/red] Not found: '{q}'")
    else:
        seeds = interactive_seed_selection(resolver)

    if not seeds:
        console.print("[bold red]No valid seeds selected. Exiting.[/bold red]")
        return

    console.print(f"\n[bold]Propagating affinity via Graph PPR (DuckDB + Audio Alignment)...[/bold]")
    with console.status("[dim]Computing Personalized PageRank and path alignments...", spinner="dots"):
        recommendations = engine.recommend(seeds, limit=10, min_popularity=35)

    if not recommendations:
        console.print("[yellow]No sufficient recommendations found with current criteria.[/yellow]")
        return

    # Results table
    table = Table(
        title="Recommendations — youmightlike",
        show_header=True,
        header_style="bold",
        border_style="dim",
    )
    table.add_column("#", justify="center", style="bold white", width=3)
    table.add_column("Track", style="bold white", min_width=24)
    table.add_column("Artist", style="cyan", min_width=18)
    table.add_column("Affinity", justify="right", style="bold green", width=11)
    table.add_column("Audio Profile", style="yellow", min_width=22)
    table.add_column("Structural Path / Rationale", style="dim white", min_width=40)

    for rank, rec in enumerate(recommendations, 1):
        score_color = "bold green" if rec.affinity_score >= 80 else "bold yellow"
        score_str = f"[{score_color}]{rec.affinity_score:.1f}%[/{score_color}]"

        # Truncate long explanations to fit cleanly
        expl = rec.explanation
        if len(expl) > 90:
            expl = expl[:87] + "..."

        table.add_row(
            str(rank),
            rec.title,
            rec.artist_name,
            score_str,
            rec.vibe_summary(),
            expl,
        )

    console.print()
    console.print(table)
    console.print(
        f"\n[dim]Computed {len(recommendations)} recommendations with verifiable graph paths.[/dim]\n"
    )


if __name__ == "__main__":
    run_cli()
