from __future__ import annotations

import logging
import time
from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TaskProgressColumn
from rich.table import Table

from youmightlike.storage.database import init_db
from youmightlike.connectors.wikidata import WikidataClient
from youmightlike.connectors.music import MusicIngestionService
from youmightlike.connectors.books import OpenLibraryClient

console = Console()

SEED_MOVIES = [
    "Blade Runner",
    "2001: A Space Odyssey",
    "Dune",
    "Interstellar",
    "The Matrix",
    "Akira",
    "Apocalypse Now",
    "Taxi Driver",
    "Drive",
    "Arrival",
    "The Shining",
    "Fight Club",
    "Solaris",
    "Spirited Away",
    "Inception",
    "Mulholland Drive",
    "Her",
    "The Social Network",
    "A Clockwork Orange",
    "Ghost in the Shell",
]

SEED_BOOKS = [
    ("Neuromancer", "William Gibson"),
    ("Dune", "Frank Herbert"),
    ("Snow Crash", "Neal Stephenson"),
    ("Fahrenheit 451", "Ray Bradbury"),
    ("1984", "George Orwell"),
    ("Brave New World", "Aldous Huxley"),
    ("Do Androids Dream of Electric Sheep", "Philip K. Dick"),
    ("Solaris", "Stanislaw Lem"),
    ("Foundation", "Isaac Asimov"),
    ("Roadside Picnic", "Arkady and Boris Strugatsky"),
]

SEED_MUSIC = [
    "Hans Zimmer",
    "Trent Reznor",
    "Ennio Morricone",
    "Wendy Carlos",
    "Joe Hisaishi",
    "Cliff Martinez",
    "Angelo Badalamenti",
    "Brian Eno",
    "Tangerine Dream",
    "Daft Punk",
    "Massive Attack",
    "Depeche Mode",
]


def run_pipeline() -> None:
    db = init_db()
    wiki_client = WikidataClient()
    music_service = MusicIngestionService()
    book_client = OpenLibraryClient()

    console.print("\n[bold cyan]🚀 Iniciando Pipeline de Ingestão em Lote (youmightlike)[/bold cyan]")
    console.print(f"📦 Alvos: {len(SEED_MOVIES)} filmes, {len(SEED_BOOKS)} livros, {len(SEED_MUSIC)} artistas/compositores\n")

    with db.connect() as conn:
        # =====================================================================
        # 1. FILMES (Wikidata)
        # =====================================================================
        console.print("[bold yellow]🎬 1/3 Ingerindo Filmes e Pontes de Trilha/Adaptação (Wikidata)...[/bold yellow]")
        composers_to_enrich = set()
        books_to_enrich = set()

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            BarColumn(),
            TaskProgressColumn(),
            console=console,
        ) as progress:
            task = progress.add_task("[yellow]Processando filmes...", total=len(SEED_MOVIES))

            for movie_name in SEED_MOVIES:
                try:
                    search_res = wiki_client.search_movies(movie_name, limit=1)
                    if search_res:
                        qid = search_res[0]["wikidata_id"]
                        details = wiki_client.get_movie_details(qid)
                        if details:
                            wiki_client.ingest_movie(details, conn)
                            for c in details.composers:
                                composers_to_enrich.add(c.name)
                            if details.based_on_title:
                                books_to_enrich.add(details.based_on_title)
                except Exception as e:
                    console.print(f"[red]Erro ao ingerir filme '{movie_name}': {e}[/red]")
                finally:
                    progress.advance(task)

        console.print(f"✅ Filmes processados! {len(composers_to_enrich)} compositores e {len(books_to_enrich)} livros-base detectados.\n")

        # =====================================================================
        # 2. LIVROS (OpenLibrary)
        # =====================================================================
        console.print("[bold blue]📚 2/3 Ingerindo Obras Literárias & Temáticas (OpenLibrary)...[/bold blue]")
        all_book_queries = [f"{t} {a}" for t, a in SEED_BOOKS] + list(books_to_enrich)
        all_book_queries = list(dict.fromkeys(all_book_queries))  # Únicos

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            BarColumn(),
            TaskProgressColumn(),
            console=console,
        ) as progress:
            task = progress.add_task("[blue]Processando livros...", total=len(all_book_queries))

            for b_query in all_book_queries:
                try:
                    books_found = book_client.search_books(b_query, limit=1)
                    if books_found:
                        book_client.ingest_book(books_found[0], conn)
                except Exception as e:
                    console.print(f"[red]Erro ao ingerir livro '{b_query}': {e}[/red]")
                finally:
                    progress.advance(task)

        console.print("✅ Livros e autores processados!\n")

        # =====================================================================
        # 3. MÚSICA (MusicBrainz)
        # =====================================================================
        console.print("[bold magenta]🎵 3/3 Ingerindo Artistas, Álbuns & Microgêneros (MusicBrainz)...[/bold magenta]")
        all_artists = list(dict.fromkeys(SEED_MUSIC + list(composers_to_enrich)))

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            BarColumn(),
            TaskProgressColumn(),
            console=console,
        ) as progress:
            task = progress.add_task("[magenta]Processando artistas...", total=len(all_artists))

            for artist_name in all_artists:
                try:
                    search_res = music_service.mb_client.search_artists(artist_name, limit=1)
                    if search_res:
                        mbid = search_res[0]["mbid"]
                        music_service.ingest_artist_by_mbid(mbid, conn)
                except Exception as e:
                    console.print(f"[red]Erro ao ingerir artista '{artist_name}': {e}[/red]")
                finally:
                    progress.advance(task)

        console.print("✅ Artistas e álbuns processados!\n")

        # =====================================================================
        # RESUMO FINAL
        # =====================================================================
        console.print("[bold green]🎉 Pipeline concluído com sucesso![/bold green]\n")

        summary_table = Table(title="📊 Censo do Grafo no DuckDB", header_style="bold green")
        summary_table.add_column("Tabela / View", style="cyan")
        summary_table.add_column("Registros", justify="right", style="bold white")

        tables = [
            ("movies", "Filmes Cadastrados"),
            ("people", "Diretores / Compositores"),
            ("movie_credits", "Créditos de Cinema"),
            ("music_artists", "Artistas Musicais"),
            ("music_albums", "Álbuns & Soundtracks"),
            ("books", "Livros Cadastrados"),
            ("book_authors", "Autores"),
            ("keywords", "Tags / Gêneros / Subjects"),
            ("v_graph_nodes", "Total de Nós no Grafo"),
            ("v_graph_edges", "Total de Arestas Ativas"),
            ("v_weighted_graph_edges", "Arestas com Pesos Adamic-Adar"),
        ]

        for t_name, label in tables:
            count = conn.execute(f"SELECT COUNT(*) FROM {t_name}").fetchone()[0]
            summary_table.add_row(f"{t_name} ({label})", str(count))

        console.print(summary_table)


if __name__ == "__main__":
    run_pipeline()
