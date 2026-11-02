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
            "[bold cyan]youmightlike[/bold cyan] — [italic green]Motor Determinístico de Recomendação Cultural[/italic green]\n"
            "[dim]40.4M Músicas • 3.2M Artistas • 18M Álbuns • Zero LLMs • 100% Local (DuckDB)[/dim]",
            border_style="cyan",
            title="🎧 Piloto Musical Transmidiático",
            subtitle="v0.2.0",
        )
    )


def interactive_seed_selection(resolver: MusicResolver) -> List[ResolvedEntity]:
    seeds: List[ResolvedEntity] = []
    console.print("\n[bold yellow]🔍 Passo 1: Escolha de 1 a 4 músicas ou artistas que você curte.[/bold yellow]")
    console.print("[dim](Pressione [Enter] em branco quando terminar de adicionar)[/dim]\n")

    while len(seeds) < 4:
        count_label = f"[{len(seeds) + 1}/4]"
        query = Prompt.ask(f"[bold cyan]Digite uma música ou artista[/bold cyan] {count_label} (ou Enter para concluir)")
        query = query.strip()
        if not query:
            if seeds:
                break
            console.print("[red]Por favor, adicione pelo menos uma semente para recomendarmos![/red]")
            continue

        console.print(f"[dim]Buscando no catálogo de 40M de faixas por '{query}'...[/dim]")
        candidates = resolver.resolve(query, limit=5)

        if not candidates:
            console.print(f"[yellow]Nenhum candidato encontrado para '{query}'. Tente outro termo.[/yellow]\n")
            continue

        # Exibe os candidatos
        console.print(f"\n[bold green]Encontrados {len(candidates)} candidatos:[/bold green]")
        for idx, c in enumerate(candidates, 1):
            console.print(f"  [bold white]{idx}[/bold white]. {c.display_label()}")

        choice_str = Prompt.ask(
            "Selecione o número correspondente (ou 0 para descartar)",
            choices=[str(i) for i in range(len(candidates) + 1)],
            default="1",
        )
        choice = int(choice_str)
        if choice > 0:
            selected = candidates[choice - 1]
            seeds.append(selected)
            console.print(f"[bold green]✔ Adicionado:[/bold green] {selected.display_label()}\n")
        else:
            console.print("[dim]Busca descartada.[/dim]\n")

    return seeds


def run_cli() -> None:
    render_banner()

    resolver = MusicResolver()
    engine = MusicRecommenderEngine()

    seeds: List[ResolvedEntity] = []

    # Se o usuário passou argumentos na linha de comando, resolve direto
    if len(sys.argv) > 1:
        raw_queries = sys.argv[1:]
        console.print(f"\n[bold yellow]🔍 Resolvendo sementes passadas por argumento: {raw_queries}[/bold yellow]")
        for q in raw_queries:
            found = resolver.resolve(q, limit=1)
            if found:
                seeds.append(found[0])
                console.print(f"  [green]✔ Reconhecido:[/green] {found[0].display_label()}")
            else:
                console.print(f"  [red]✖ Não encontrado:[/red] '{q}'")
    else:
        seeds = interactive_seed_selection(resolver)

    if not seeds:
        console.print("[bold red]Nenhuma semente válida selecionada. Encerrando.[/bold red]")
        return

    console.print(f"\n[bold cyan]🧠 Difundindo afinidade pelo Grafo de Gosto (Last.fm + Tags + Audio Features)...[/bold cyan]")
    with console.status("[bold green]Executando Random Walk ponderado e alinhamento acústico...", spinner="dots"):
        recommendations = engine.recommend(seeds, limit=10, min_popularity=35)

    if not recommendations:
        console.print("[yellow]Não encontramos recomendações suficientes com os critérios atuais.[/yellow]")
        return

    # Tabela com resultados
    table = Table(
        title="✨ Recomendações Personalizadas — youmightlike",
        show_header=True,
        header_style="bold magenta",
        border_style="cyan",
    )
    table.add_column("#", justify="center", style="bold white", width=3)
    table.add_column("Música", style="bold white", min_width=24)
    table.add_column("Artista", style="cyan", min_width=18)
    table.add_column("Afinidade", justify="right", style="bold green", width=11)
    table.add_column("Áudio Vibe", style="yellow", min_width=22)
    table.add_column("Por que você pode curtir? (Justificativa)", style="dim white", min_width=40)

    for rank, rec in enumerate(recommendations, 1):
        score_color = "bold green" if rec.affinity_score >= 80 else "bold yellow"
        score_str = f"[{score_color}]{rec.affinity_score:.1f}%[/{score_color}]"

        # Trunca explicação longa para caber bem
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
        f"\n[bold green]✅ 10 recomendações calculadas com sucesso![/bold green] "
        f"[dim]Todas as sugestões possuem trilha de conexão estrutural auditável.[/dim]\n"
    )


if __name__ == "__main__":
    run_cli()
