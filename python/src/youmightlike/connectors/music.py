from __future__ import annotations

import logging
import os
import time
from typing import Any
import httpx
from pydantic import BaseModel, Field
import duckdb
from dotenv import load_dotenv

load_dotenv()

logger = logging.getLogger(__name__)

MUSICBRAINZ_API_URL = "https://musicbrainz.org/ws/2"
LASTFM_API_URL = "https://ws.audioscrobbler.com/2.0/"
USER_AGENT = "YouMightLike/0.1.0 ( lucasnovack@gmail.com )"


class AlbumRef(BaseModel):
    mbid: str | None = None
    title: str
    release_year: int | None = None
    primary_type: str = "Album"


class TagRef(BaseModel):
    name: str
    weight: float = 1.0


class SimilarArtistRef(BaseModel):
    name: str
    match_score: float = 0.5
    mbid: str | None = None


class ArtistDetails(BaseModel):
    mbid: str
    wikidata_id: str | None = None
    name: str
    country: str | None = None
    artist_type: str | None = None
    albums: list[AlbumRef] = Field(default_factory=list)
    tags: list[TagRef] = Field(default_factory=list)
    similar_artists: list[SimilarArtistRef] = Field(default_factory=list)


class MusicBrainzClient:
    """Cliente para a API aberta e gratuita do MusicBrainz (100% aberta, sem chaves)."""

    def __init__(self, timeout: float = 15.0):
        self.timeout = timeout
        self.headers = {
            "User-Agent": USER_AGENT,
            "Accept": "application/json",
        }
        self._last_request_time = 0.0

    def _rate_limit(self) -> None:
        """Respeita o limite estrito de 1 req/s do MusicBrainz."""
        elapsed = time.time() - self._last_request_time
        if elapsed < 1.1:
            time.sleep(1.1 - elapsed)
        self._last_request_time = time.time()

    def _get(self, url: str, params: dict[str, Any], max_retries: int = 3) -> dict[str, Any]:
        """Executa requisição GET com rate limiting e retry automático para 503."""
        for attempt in range(max_retries):
            self._rate_limit()
            with httpx.Client(timeout=self.timeout) as client:
                resp = client.get(url, params=params, headers=self.headers)
                if resp.status_code == 200:
                    return resp.json()
                elif resp.status_code == 503:
                    time.sleep(2.0 * (attempt + 1))
                    continue
                resp.raise_for_status()
        raise RuntimeError(f"MusicBrainz API falhou após {max_retries} tentativas: {url}")

    def search_artists(self, query: str, limit: int = 5) -> list[dict[str, Any]]:
        """Pesquisa artistas no MusicBrainz pelo nome."""
        params = {"query": f"artist:{query}", "fmt": "json", "limit": limit}
        data = self._get(f"{MUSICBRAINZ_API_URL}/artist", params=params)

        artists = []
        for item in data.get("artists", []):
            artists.append({
                "mbid": item["id"],
                "name": item["name"],
                "type": item.get("type"),
                "country": item.get("country"),
                "score": item.get("score"),
                "disambiguation": item.get("disambiguation", ""),
            })
        return artists

    def get_artist_details(self, mbid: str) -> ArtistDetails | None:
        """Puxa a ficha completa do artista no MusicBrainz: álbuns, tags e cross-link para o Wikidata."""
        self._rate_limit()
        params = {
            "inc": "release-groups+tags+genres+url-rels",
            "fmt": "json",
        }
        try:
            data = self._get(f"{MUSICBRAINZ_API_URL}/artist/{mbid}", params=params)
        except Exception:
            return None

        # Extrair link para o Wikidata se houver
        wikidata_id: str | None = None
        for rel in data.get("relations", []):
            if rel.get("type") == "wikidata":
                resource = rel.get("url", {}).get("resource", "")
                if "/wiki/Q" in resource:
                    wikidata_id = resource.split("/")[-1]
                    break

        # Extrair álbuns e trilhas sonoras principais
        albums: list[AlbumRef] = []
        seen_titles = set()
        for rg in data.get("release-groups", []):
            title = rg.get("title")
            p_type = rg.get("primary-type") or "Album"
            if title and title.lower() not in seen_titles:
                seen_titles.add(title.lower())
                first_release = rg.get("first-release-date", "")
                year = None
                if first_release and len(first_release) >= 4:
                    try:
                        year = int(first_release[:4])
                    except ValueError:
                        pass

                albums.append(AlbumRef(
                    mbid=rg.get("id"),
                    title=title,
                    release_year=year,
                    primary_type=p_type,
                ))

        # Extrair tags e gêneros
        tags: list[TagRef] = []
        seen_tags = set()
        # 1. Gêneros formais do MusicBrainz
        for g in data.get("genres", []):
            g_name = g.get("name", "").lower()
            if g_name and g_name not in seen_tags:
                seen_tags.add(g_name)
                tags.append(TagRef(name=g_name, weight=1.5))

        # 2. Tags comunitárias
        for t in data.get("tags", []):
            t_name = t.get("name", "").lower()
            if t_name and t_name not in seen_tags:
                seen_tags.add(t_name)
                count = t.get("count", 1)
                normalized_weight = min(2.0, max(0.5, count / 5.0))
                tags.append(TagRef(name=t_name, weight=round(normalized_weight, 2)))

        return ArtistDetails(
            mbid=mbid,
            wikidata_id=wikidata_id,
            name=data["name"],
            country=data.get("country"),
            artist_type=data.get("type"),
            albums=albums,
            tags=tags,
        )


class LastFMClient:
    """Cliente para a API do Last.fm (Folksonomia de tags e similaridade)."""

    def __init__(self, api_key: str | None = None, timeout: float = 10.0):
        self.api_key = api_key or os.getenv("LASTFM_API_KEY")
        self.timeout = timeout

    @property
    def is_configured(self) -> bool:
        return bool(self.api_key and "sua_chave" not in self.api_key)

    def get_top_tags(self, artist_name: str, limit: int = 10) -> list[TagRef]:
        """Extrai tags e humores do Last.fm para o artista."""
        if not self.is_configured:
            return []

        params = {
            "method": "artist.gettoptags",
            "artist": artist_name,
            "api_key": self.api_key,
            "format": "json",
            "autocorrect": 1,
        }
        try:
            with httpx.Client(timeout=self.timeout) as client:
                resp = client.get(LASTFM_API_URL, params=params)
                if resp.status_code != 200:
                    return []
                data = resp.json()
        except Exception:
            return []

        tags = []
        for t in data.get("toptags", {}).get("tag", [])[:limit]:
            name = t.get("name", "").lower()
            count = t.get("count", 100)
            if name:
                weight = round(min(2.0, max(0.2, count / 50.0)), 2)
                tags.append(TagRef(name=name, weight=weight))
        return tags

    def get_similar_artists(self, artist_name: str, limit: int = 5) -> list[SimilarArtistRef]:
        """Extrai artistas esteticamente similares calculados pelo Last.fm."""
        if not self.is_configured:
            return []

        params = {
            "method": "artist.getsimilar",
            "artist": artist_name,
            "api_key": self.api_key,
            "format": "json",
            "limit": limit,
            "autocorrect": 1,
        }
        try:
            with httpx.Client(timeout=self.timeout) as client:
                resp = client.get(LASTFM_API_URL, params=params)
                if resp.status_code != 200:
                    return []
                data = resp.json()
        except Exception:
            return []

        similar = []
        for a in data.get("similarartists", {}).get("artist", []):
            name = a.get("name")
            match = float(a.get("match", 0.5))
            mbid = a.get("mbid") or None
            if name:
                similar.append(SimilarArtistRef(name=name, match_score=round(match, 3), mbid=mbid))
        return similar


class MusicIngestionService:
    """Orquestrador de enriquecimento e persistência de dados musicais no DuckDB."""

    def __init__(self):
        self.mb_client = MusicBrainzClient()
        self.lastfm_client = LastFMClient()

    def ingest_artist_by_mbid(self, mbid: str, conn: duckdb.DuckDBPyConnection) -> int:
        """Puxa ficha do MusicBrainz (+ Last.fm se configurado) e persiste no DuckDB."""
        details = self.mb_client.get_artist_details(mbid)
        if not details:
            raise ValueError(f"Artista não encontrado no MusicBrainz para o MBID: {mbid}")

        # Enriquecimento opcional via Last.fm
        if self.lastfm_client.is_configured:
            extra_tags = self.lastfm_client.get_top_tags(details.name)
            existing_tag_names = {t.name for t in details.tags}
            for t in extra_tags:
                if t.name not in existing_tag_names:
                    details.tags.append(t)

            details.similar_artists = self.lastfm_client.get_similar_artists(details.name)

        # 1. Inserir ou atualizar em music_artists
        existing = conn.execute(
            "SELECT id, wikidata_id FROM music_artists WHERE mbid = ?", [details.mbid]
        ).fetchone()

        if existing:
            artist_id = existing[0]
            if not existing[1] and details.wikidata_id:
                try:
                    conn.execute(
                        "UPDATE music_artists SET wikidata_id = ? WHERE id = ?",
                        [details.wikidata_id, artist_id],
                    )
                except Exception:
                    pass
        else:
            artist_id = conn.execute(
                """
                INSERT INTO music_artists (mbid, wikidata_id, name, country, type)
                VALUES (?, ?, ?, ?, ?)
                RETURNING id
                """,
                [details.mbid, details.wikidata_id, details.name, details.country, details.artist_type],
            ).fetchone()[0]

        # 2. Inserir Álbuns
        for alb in details.albums:
            conn.execute(
                """
                INSERT INTO music_albums (mbid, artist_id, title, release_year, primary_type)
                SELECT ?, ?, ?, ?, ?
                WHERE NOT EXISTS (
                    SELECT 1 FROM music_albums WHERE artist_id = ? AND LOWER(title) = LOWER(?)
                )
                """,
                [alb.mbid, artist_id, alb.title, alb.release_year, alb.primary_type, artist_id, alb.title],
            )

        # 3. Inserir Tags / Gêneros
        for tag in details.tags:
            kw_existing = conn.execute(
                "SELECT id FROM keywords WHERE name = ?", [tag.name]
            ).fetchone()
            if kw_existing:
                kw_id = kw_existing[0]
            else:
                kw_id = conn.execute(
                    "INSERT INTO keywords (name, source) VALUES (?, 'musicbrainz') RETURNING id",
                    [tag.name],
                ).fetchone()[0]

            conn.execute(
                """
                INSERT INTO artist_tags (artist_id, keyword_id, weight)
                VALUES (?, ?, ?)
                ON CONFLICT (artist_id, keyword_id) DO UPDATE SET weight = EXCLUDED.weight
                """,
                [artist_id, kw_id, tag.weight],
            )

        return artist_id


if __name__ == "__main__":
    from youmightlike.storage.database import init_db

    db = init_db()
    service = MusicIngestionService()

    print("Pesquisando 'Vangelis' no MusicBrainz...")
    results = service.mb_client.search_artists("Vangelis", limit=3)
    for r in results:
        print(f" -> {r['mbid']}: {r['name']} ({r.get('type')}, {r.get('country')})")

    if results:
        vangelis_mbid = results[0]["mbid"]
        print(f"\nIngerindo '{results[0]['name']}' ({vangelis_mbid}) no DuckDB...")
        with db.connect() as conn:
            artist_id = service.ingest_artist_by_mbid(vangelis_mbid, conn)
            print(f"Artista inserido com sucesso! ID Interno: {artist_id}")

            # Consultar nós e arestas no Grafo
            nodes_count = conn.execute("SELECT COUNT(*) FROM v_graph_nodes").fetchone()[0]
            edges_count = conn.execute("SELECT COUNT(*) FROM v_graph_edges").fetchone()[0]
            print(f"\n️ Total de Nós no Grafo: {nodes_count}")
            print(f"️ Total de Arestas no Grafo: {edges_count}")

            # Mostrar a ponte transmidiática ao vivo!
            print("\nPontes Transmidiáticas Criadas (Filme ⇄ Música):")
            bridges = conn.execute(
                """
                SELECT source_id, relation_type, target_id, base_weight 
                FROM v_graph_edges 
                WHERE relation_type IN ('has_composer', 'is_artist', 'released')
                ORDER BY relation_type DESC
                LIMIT 10
                """
            ).fetchall()
            for b in bridges:
                print(f"  {b[0]} --[{b[1]}]--> {b[2]} (peso {b[3]})")
