from __future__ import annotations

import logging
from typing import Any
import httpx
from pydantic import BaseModel, Field
import duckdb

logger = logging.getLogger(__name__)

OPENLIBRARY_SEARCH_URL = "https://openlibrary.org/search.json"
OPENLIBRARY_WORKS_URL = "https://openlibrary.org/works"
USER_AGENT = "YouMightLike/0.1.0 ( lucasnovack@gmail.com; open-source-project )"

# Tags genéricas ou estruturais do OpenLibrary que devem ser descartadas
IGNORED_SUBJECTS = {
    "open_syllabus_project",
    "large type books",
    "protected daisy",
    "accessible book",
    "in library",
    "lending library",
    "internet archive wishlist",
    "new york times reviewed",
    "overdrive",
}


class AuthorRef(BaseModel):
    author_key: str | None = None
    name: str


class BookDetails(BaseModel):
    work_key: str
    wikidata_id: str | None = None
    title: str
    first_publish_year: int | None = None
    authors: list[AuthorRef] = Field(default_factory=list)
    subjects: list[str] = Field(default_factory=list)
    description: str | None = None


class OpenLibraryClient:
    """Cliente para a API aberta do OpenLibrary (Internet Archive - 100% gratuita e sem token)."""

    def __init__(self, timeout: float = 15.0):
        self.timeout = timeout
        self.headers = {
            "User-Agent": USER_AGENT,
            "Accept": "application/json",
        }

    def search_books(self, query: str, limit: int = 5) -> list[BookDetails]:
        """Pesquisa obras no OpenLibrary trazendo título, autores, ano e assuntos."""
        params = {
            "q": query,
            "fields": "key,title,author_name,author_key,first_publish_year,subject",
            "limit": limit,
        }

        with httpx.Client(timeout=self.timeout) as client:
            resp = client.get(OPENLIBRARY_SEARCH_URL, params=params, headers=self.headers)
            resp.raise_for_status()
            docs = resp.json().get("docs", [])

        books = []
        for doc in docs:
            work_key = doc.get("key", "")
            title = doc.get("title", "")
            if not work_key or not title:
                continue

            authors = []
            author_names = doc.get("author_name", [])
            author_keys = doc.get("author_key", [])
            for idx, name in enumerate(author_names):
                a_key = author_keys[idx] if idx < len(author_keys) else None
                authors.append(AuthorRef(author_key=a_key, name=name))

            subjects = []
            for s in doc.get("subject", []):
                cleaned = s.strip().lower()
                if cleaned and cleaned not in IGNORED_SUBJECTS and len(cleaned) < 50:
                    subjects.append(cleaned)

            books.append(BookDetails(
                work_key=work_key,
                title=title,
                first_publish_year=doc.get("first_publish_year"),
                authors=authors,
                subjects=list(dict.fromkeys(subjects))[:15],  # Top 15 únicos
            ))

        return books

    def get_work_details(self, work_id: str) -> BookDetails | None:
        """Puxa os detalhes direto do endpoint /works/{id}.json."""
        clean_id = work_id.replace("/works/", "")
        url = f"{OPENLIBRARY_WORKS_URL}/{clean_id}.json"

        with httpx.Client(timeout=self.timeout) as client:
            resp = client.get(url, headers=self.headers)
            if resp.status_code == 404:
                return None
            resp.raise_for_status()
            data = resp.json()

        title = data.get("title", "")
        desc = data.get("description", "")
        if isinstance(desc, dict):
            desc = desc.get("value", "")

        subjects = []
        for s in data.get("subjects", []):
            cleaned = s.strip().lower()
            if cleaned and cleaned not in IGNORED_SUBJECTS:
                subjects.append(cleaned)

        return BookDetails(
            work_key=f"/works/{clean_id}",
            title=title,
            description=desc[:500] if desc else None,
            subjects=list(dict.fromkeys(subjects))[:15],
        )

    def ingest_book(self, book: BookDetails, conn: duckdb.DuckDBPyConnection) -> int:
        """Persiste um livro, seus autores e tags/subjects no DuckDB de forma atômica."""
        # 1. Verificar se o livro já existe (por work_key ou pelo título já criado por adaptação)
        existing = conn.execute(
            """
            SELECT id, work_key FROM books 
            WHERE work_key = ? 
               OR (work_key IS NULL AND LOWER(TRIM(title)) = LOWER(TRIM(?)))
            """,
            [book.work_key, book.title],
        ).fetchone()

        if existing:
            book_id = existing[0]
            if not existing[1] and book.work_key:
                try:
                    conn.execute(
                        "UPDATE books SET work_key = ?, first_publish_year = COALESCE(first_publish_year, ?) WHERE id = ?",
                        [book.work_key, book.first_publish_year, book_id],
                    )
                except Exception:
                    pass
        else:
            book_id = conn.execute(
                """
                INSERT INTO books (work_key, title, first_publish_year, description)
                VALUES (?, ?, ?, ?)
                RETURNING id
                """,
                [book.work_key, book.title, book.first_publish_year, book.description],
            ).fetchone()[0]

        # 2. Inserir Autores e Vínculos
        for author in book.authors:
            a_existing = conn.execute(
                """
                SELECT id FROM book_authors 
                WHERE (author_key IS NOT NULL AND author_key = ?) 
                   OR LOWER(TRIM(name)) = LOWER(TRIM(?))
                """,
                [author.author_key, author.name],
            ).fetchone()

            if a_existing:
                author_id = a_existing[0]
            else:
                author_id = conn.execute(
                    "INSERT INTO book_authors (author_key, name) VALUES (?, ?) RETURNING id",
                    [author.author_key, author.name],
                ).fetchone()[0]

            conn.execute(
                """
                INSERT INTO book_author_links (book_id, author_id)
                VALUES (?, ?)
                ON CONFLICT DO NOTHING
                """,
                [book_id, author_id],
            )

        # 3. Inserir Subjects como Keywords no Grafo
        for subj in book.subjects:
            kw_existing = conn.execute(
                "SELECT id FROM keywords WHERE name = ?", [subj]
            ).fetchone()
            if kw_existing:
                kw_id = kw_existing[0]
            else:
                kw_id = conn.execute(
                    "INSERT INTO keywords (name, source) VALUES (?, 'openlibrary') RETURNING id",
                    [subj],
                ).fetchone()[0]

            conn.execute(
                """
                INSERT INTO book_subjects (book_id, keyword_id)
                VALUES (?, ?)
                ON CONFLICT DO NOTHING
                """,
                [book_id, kw_id],
            )

        return book_id


if __name__ == "__main__":
    from youmightlike.storage.database import init_db

    db = init_db()
    client = OpenLibraryClient()

    # 1. Enriquecer o livro de Blade Runner que já estava pré-criado no DuckDB
    print("🔎 Pesquisando 'Do Androids Dream of Electric Sheep' no OpenLibrary...")
    blade_runner_books = client.search_books("Do Androids Dream of Electric Sheep", limit=1)
    if blade_runner_books:
        book = blade_runner_books[0]
        print(f" -> Título: {book.title} ({book.first_publish_year})")
        print(f" -> Autores: {[a.name for a in book.authors]}")
        print(f" -> Subjects: {book.subjects[:5]}")

        with db.connect() as conn:
            book_id = client.ingest_book(book, conn)
            print(f"🎉 Livro enriquecido com sucesso! ID Interno: {book_id}")

    # 2. Inserir um segundo livro clássico: Neuromancer de William Gibson
    print("\n🔎 Pesquisando e ingerindo 'Neuromancer'...")
    neuromancer_books = client.search_books("Neuromancer William Gibson", limit=1)
    if neuromancer_books:
        neuro = neuromancer_books[0]
        print(f" -> Título: {neuro.title} ({neuro.first_publish_year})")
        print(f" -> Autores: {[a.name for a in neuro.authors]}")
        print(f" -> Subjects: {neuro.subjects[:5]}")

        with db.connect() as conn:
            neuro_id = client.ingest_book(neuro, conn)
            print(f"🎉 Livro inserido! ID Interno: {neuro_id}")

            # Estado consolidado do Grafo Transmidiático!
            nodes_count = conn.execute("SELECT COUNT(*) FROM v_graph_nodes").fetchone()[0]
            edges_count = conn.execute("SELECT COUNT(*) FROM v_graph_edges").fetchone()[0]
            print(f"\n🕸️ Total de Nós no Grafo: {nodes_count}")
            print(f"🕸️ Total de Arestas no Grafo: {edges_count}")

            print("\n🌟 Pontes Transmidiáticas Ativas no DuckDB (Filme ⇄ Música ⇄ Livro):")
            bridges = conn.execute(
                """
                SELECT source_id, relation_type, target_id, base_weight 
                FROM v_graph_edges 
                WHERE relation_type IN ('has_composer', 'is_artist', 'based_on', 'written_by', 'has_subject')
                ORDER BY relation_type DESC
                LIMIT 15
                """
            ).fetchall()
            for b in bridges:
                print(f"  {b[0]} --[{b[1]}]--> {b[2]} (peso {b[3]})")
