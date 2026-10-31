from __future__ import annotations

import logging
from typing import Any
import httpx
from pydantic import BaseModel, Field
import duckdb

logger = logging.getLogger(__name__)

WIKIDATA_SPARQL_URL = "https://query.wikidata.org/sparql"
WIKIDATA_API_URL = "https://www.wikidata.org/w/api.php"
USER_AGENT = "YouMightLike/0.1.0 (https://github.com/lucasnovack/youmightlike; open-source-project)"


class PersonRef(BaseModel):
    wikidata_id: str
    name: str


class MovieDetails(BaseModel):
    wikidata_id: str
    title: str
    year: int | None = None
    directors: list[PersonRef] = Field(default_factory=list)
    composers: list[PersonRef] = Field(default_factory=list)
    writers: list[PersonRef] = Field(default_factory=list)
    genres: list[str] = Field(default_factory=list)
    subjects: list[str] = Field(default_factory=list)
    based_on_title: str | None = None
    based_on_wikidata_id: str | None = None


class WikidataClient:
    """Cliente para busca e extração de relações transmidiáticas do Wikidata."""

    def __init__(self, timeout: float = 20.0):
        self.timeout = timeout
        self.headers = {
            "User-Agent": USER_AGENT,
            "Accept": "application/sparql-results+json",
        }

    def search_movies(self, query: str, limit: int = 5) -> list[dict[str, str]]:
        """Busca filmes no Wikidata pelo nome e retorna sugestões com QID e descrição."""
        params = {
            "action": "wbsearchentities",
            "search": query,
            "language": "en",
            "type": "item",
            "format": "json",
            "limit": limit * 2,
        }
        with httpx.Client(timeout=self.timeout) as client:
            resp = client.get(WIKIDATA_API_URL, params=params, headers=self.headers)
            resp.raise_for_status()
            results = resp.json().get("search", [])

        movies = []
        for item in results:
            desc = item.get("description", "").lower()
            # Prioriza itens que são descritos como filmes/filmes de tv
            if any(term in desc for term in ["film", "movie", "cinema", "motion picture", "directed by"]):
                movies.append({
                    "wikidata_id": item["id"],
                    "title": item.get("match", {}).get("text", item["label"]),
                    "description": item.get("description", ""),
                })
            if len(movies) >= limit:
                break

        # Se nenhum teve a palavra mágica na descrição, retorna os primeiros gerais
        if not movies and results:
            for item in results[:limit]:
                movies.append({
                    "wikidata_id": item["id"],
                    "title": item["label"],
                    "description": item.get("description", ""),
                })

        return movies

    def get_movie_details(self, wikidata_id: str) -> MovieDetails | None:
        """Extrai a teia completa de um filme no Wikidata: diretores, compositores, gêneros e livro base."""
        query = f"""
        SELECT ?movie ?movieLabel ?year ?director ?directorLabel ?composer ?composerLabel ?writer ?writerLabel ?genre ?genreLabel ?subject ?subjectLabel ?basedOn ?basedOnLabel WHERE {{
          VALUES ?movie {{ wd:{wikidata_id} }}
          OPTIONAL {{ ?movie wdt:P57 ?director. }}
          OPTIONAL {{ ?movie wdt:P86 ?composer. }}
          OPTIONAL {{ ?movie wdt:P58 ?writer. }}
          OPTIONAL {{ ?movie wdt:P136 ?genre. }}
          OPTIONAL {{ ?movie wdt:P921 ?subject. }}
          OPTIONAL {{ ?movie wdt:P144 ?basedOn. }}
          OPTIONAL {{ 
            ?movie wdt:P577 ?pubDate.
            BIND(YEAR(?pubDate) AS ?year)
          }}
          SERVICE wikibase:label {{ bd:serviceParam wikibase:language "en". }}
        }}
        """
        with httpx.Client(timeout=self.timeout) as client:
            resp = client.get(
                WIKIDATA_SPARQL_URL,
                params={"query": query},
                headers=self.headers,
            )
            resp.raise_for_status()
            rows = resp.json().get("results", {}).get("bindings", [])

        if not rows:
            return None

        directors_map: dict[str, str] = {}
        composers_map: dict[str, str] = {}
        writers_map: dict[str, str] = {}
        genres_set: set[str] = set()
        subjects_set: set[str] = set()
        title = ""
        year: int | None = None
        based_on_id: str | None = None
        based_on_title: str | None = None

        for row in rows:
            if not title and "movieLabel" in row:
                title = row["movieLabel"]["value"]
            if year is None and "year" in row:
                try:
                    year = int(row["year"]["value"])
                except (ValueError, TypeError):
                    pass
            if not based_on_id and "basedOn" in row:
                based_on_id = row["basedOn"]["value"].split("/")[-1]
                based_on_title = row.get("basedOnLabel", {}).get("value")

            if "director" in row and "directorLabel" in row:
                d_id = row["director"]["value"].split("/")[-1]
                directors_map[d_id] = row["directorLabel"]["value"]

            if "composer" in row and "composerLabel" in row:
                c_id = row["composer"]["value"].split("/")[-1]
                composers_map[c_id] = row["composerLabel"]["value"]

            if "writer" in row and "writerLabel" in row:
                w_id = row["writer"]["value"].split("/")[-1]
                writers_map[w_id] = row["writerLabel"]["value"]

            if "genreLabel" in row:
                genres_set.add(row["genreLabel"]["value"].lower())

            if "subjectLabel" in row:
                subjects_set.add(row["subjectLabel"]["value"].lower())

        return MovieDetails(
            wikidata_id=wikidata_id,
            title=title or wikidata_id,
            year=year,
            directors=[PersonRef(wikidata_id=k, name=v) for k, v in directors_map.items()],
            composers=[PersonRef(wikidata_id=k, name=v) for k, v in composers_map.items()],
            writers=[PersonRef(wikidata_id=k, name=v) for k, v in writers_map.items()],
            genres=sorted(genres_set),
            subjects=sorted(subjects_set),
            based_on_title=based_on_title,
            based_on_wikidata_id=based_on_id,
        )

    def ingest_movie(self, details: MovieDetails, conn: duckdb.DuckDBPyConnection) -> int:
        """Persiste um filme e todas as suas arestas relacionais no DuckDB de forma atômica."""
        # 1. Inserir ou atualizar filme
        existing = conn.execute(
            "SELECT id FROM movies WHERE wikidata_id = ?", [details.wikidata_id]
        ).fetchone()

        if existing:
            movie_id = existing[0]
            conn.execute(
                """
                UPDATE movies 
                SET title = ?, release_year = ?, based_on_title = ?, based_on_wikidata_id = ?
                WHERE id = ?
                """,
                [details.title, details.year, details.based_on_title, details.based_on_wikidata_id, movie_id],
            )
        else:
            movie_id = conn.execute(
                """
                INSERT INTO movies (wikidata_id, title, release_year, based_on_title, based_on_wikidata_id)
                VALUES (?, ?, ?, ?, ?)
                RETURNING id
                """,
                [details.wikidata_id, details.title, details.year, details.based_on_title, details.based_on_wikidata_id],
            ).fetchone()[0]

        # 2. Inserir e vincular pessoas (Diretores, Compositores, Roteiristas)
        roles = [
            (details.directors, "Director", "Directing"),
            (details.composers, "Original Music Composer", "Sound"),
            (details.writers, "Screenplay", "Writing"),
        ]

        for people_list, job, department in roles:
            for p in people_list:
                p_existing = conn.execute(
                    "SELECT id FROM people WHERE wikidata_id = ?", [p.wikidata_id]
                ).fetchone()
                if p_existing:
                    person_id = p_existing[0]
                else:
                    person_id = conn.execute(
                        "INSERT INTO people (wikidata_id, name) VALUES (?, ?) RETURNING id",
                        [p.wikidata_id, p.name],
                    ).fetchone()[0]

                # Vincular créditos
                conn.execute(
                    """
                    INSERT INTO movie_credits (movie_id, person_id, job, department)
                    SELECT ?, ?, ?, ?
                    WHERE NOT EXISTS (
                        SELECT 1 FROM movie_credits 
                        WHERE movie_id = ? AND person_id = ? AND job = ?
                    )
                    """,
                    [movie_id, person_id, job, department, movie_id, person_id, job],
                )

        # 3. Inserir Gêneros
        for genre_name in details.genres:
            g_existing = conn.execute(
                "SELECT id FROM genres WHERE name = ?", [genre_name]
            ).fetchone()
            if g_existing:
                genre_id = g_existing[0]
            else:
                genre_id = conn.execute(
                    "INSERT INTO genres (name) VALUES (?) RETURNING id", [genre_name]
                ).fetchone()[0]

            conn.execute(
                """
                INSERT INTO movie_genres (movie_id, genre_id)
                VALUES (?, ?)
                ON CONFLICT DO NOTHING
                """,
                [movie_id, genre_id],
            )

        # 4. Inserir Keywords / Subjects
        for kw_name in details.subjects:
            kw_existing = conn.execute(
                "SELECT id FROM keywords WHERE name = ?", [kw_name]
            ).fetchone()
            if kw_existing:
                kw_id = kw_existing[0]
            else:
                kw_id = conn.execute(
                    "INSERT INTO keywords (name, source) VALUES (?, 'wikidata') RETURNING id",
                    [kw_name],
                ).fetchone()[0]

            conn.execute(
                """
                INSERT INTO movie_keywords (movie_id, keyword_id)
                VALUES (?, ?)
                ON CONFLICT DO NOTHING
                """,
                [movie_id, kw_id],
            )

        # 5. Se o filme é baseado em livro/obra, já cria o nó em books para a ponte direta!
        if details.based_on_wikidata_id and details.based_on_title:
            b_existing = conn.execute(
                "SELECT id FROM books WHERE wikidata_id = ?", [details.based_on_wikidata_id]
            ).fetchone()
            if not b_existing:
                conn.execute(
                    """
                    INSERT INTO books (wikidata_id, title)
                    VALUES (?, ?)
                    ON CONFLICT DO NOTHING
                    """,
                    [details.based_on_wikidata_id, details.based_on_title],
                )

        return movie_id


if __name__ == "__main__":
    from youmightlike.storage.database import init_db

    db = init_db()
    client = WikidataClient()

    print("🔎 Buscando filme 'Blade Runner' no Wikidata...")
    search_results = client.search_movies("Blade Runner", limit=3)
    for res in search_results:
        print(f" -> {res['wikidata_id']}: {res['title']} ({res['description']})")

    if search_results:
        target_qid = search_results[0]["wikidata_id"]
        print(f"\n📥 Extraindo teia do filme {target_qid}...")
        details = client.get_movie_details(target_qid)
        if details:
            print(f"Título: {details.title} ({details.year})")
            print(f"Diretores: {[d.name for d in details.directors]}")
            print(f"Compositores (Trilha): {[c.name for c in details.composers]}")
            print(f"Roteiristas: {[w.name for w in details.writers]}")
            print(f"Gêneros: {details.genres[:5]}")
            print(f"Subjects: {details.subjects}")
            if details.based_on_title:
                print(f"Livro base: {details.based_on_title} ({details.based_on_wikidata_id})")

            with db.connect() as conn:
                movie_id = client.ingest_movie(details, conn)
                print(f"\n🎉 Filme inserido no DuckDB com ID interno: {movie_id}!")

                # Inspecionar as views de grafo geradas
                print("\n🕸️ Estado das Views do Grafo no DuckDB:")
                nodes_count = conn.execute("SELECT COUNT(*) FROM v_graph_nodes").fetchone()[0]
                edges_count = conn.execute("SELECT COUNT(*) FROM v_graph_edges").fetchone()[0]
                print(f"Nós no grafo (v_graph_nodes): {nodes_count}")
                print(f"Arestas no grafo (v_graph_edges): {edges_count}")

                print("\nExemplo das arestas criadas:")
                sample_edges = conn.execute(
                    "SELECT source_id, relation_type, target_id, base_weight FROM v_graph_edges LIMIT 10"
                ).fetchall()
                for e in sample_edges:
                    print(f"  {e[0]} --[{e[1]}]--> {e[2]} (peso {e[3]})")
