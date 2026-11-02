from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional
import duckdb


@dataclass
class ResolvedEntity:
    node_id: str
    entity_type: str  # 'track' | 'artist'
    title: str
    artist_name: Optional[str] = None
    album_name: Optional[str] = None
    popularity: int = 0
    tempo: Optional[float] = None
    valence: Optional[float] = None
    energy: Optional[float] = None
    danceability: Optional[float] = None
    acousticness: Optional[float] = None
    instrumentalness: Optional[float] = None
    country: Optional[str] = None

    def display_label(self) -> str:
        if self.entity_type == "track":
            vibe = []
            if self.tempo:
                vibe.append(f"{int(round(self.tempo))} BPM")
            if self.valence is not None:
                vibe.append("Alegre/Upbeat" if self.valence >= 0.5 else "Melancólico/Sombrio")
            if self.energy is not None:
                vibe.append(f"Energia: {int(self.energy * 100)}%")
            vibe_str = f" [{', '.join(vibe)}]" if vibe else ""
            return f"🎵 {self.title} — [bold cyan]{self.artist_name}[/bold cyan] (Pop: {self.popularity}){vibe_str}"
        else:
            loc = f" ({self.country})" if self.country else ""
            return f"🎤 [bold green][Artista][/bold green] {self.title}{loc}"


class MusicResolver:
    """Resolvedor de entidades musicais de alta velocidade e precisão sobre o DuckDB."""

    def __init__(self, db_path: str = "data/processed/taste_graph.duckdb") -> None:
        self.db_path = db_path

    def _get_connection(self) -> duckdb.DuckDBPyConnection:
        conn = duckdb.connect(self.db_path, read_only=True)
        conn.execute("PRAGMA threads=8;")
        return conn

    def resolve(self, query: str, limit: int = 6) -> List[ResolvedEntity]:
        """Resolve uma busca em texto livre para candidatos estruturados no grafo em milissegundos."""
        q = query.strip()
        if not q or len(q) < 2:
            return []

        q_lower = q.lower()
        results: List[ResolvedEntity] = []
        seen_ids = set()

        variants = list(dict.fromkeys([
            q,
            q.title(),
            q.capitalize(),
            q_lower,
            q.upper()
        ]))

        with self._get_connection() as conn:
            # -----------------------------------------------------------------
            # 1. BUSCA POR ARTISTA (Usa idx_artists_name)
            # -----------------------------------------------------------------
            placeholders_art = " OR ".join(["name = ? OR name LIKE ? || '%'" for _ in variants])
            params_art = []
            for v in variants:
                params_art.extend([v, v])

            sql_artist = f"""
            SELECT 
                'artist:' || CAST(id AS VARCHAR) AS node_id,
                'artist' AS entity_type,
                name AS title,
                name AS artist_name,
                NULL AS album_name,
                COALESCE(listeners, 0) AS popularity,
                NULL AS tempo,
                NULL AS valence,
                NULL AS energy,
                NULL AS danceability,
                NULL AS acousticness,
                NULL AS instrumentalness,
                country
            FROM music_artists
            WHERE {placeholders_art}
            ORDER BY 
                CASE WHEN LOWER(name) = ? THEN 1 ELSE 2 END,
                LENGTH(name) ASC,
                listeners DESC
            LIMIT 3;
            """
            for row in conn.execute(sql_artist, params_art + [q_lower]).fetchall():
                node_id = row[0]
                if node_id not in seen_ids:
                    seen_ids.add(node_id)
                    results.append(ResolvedEntity(*row))

            # -----------------------------------------------------------------
            # 2. SE O USUÁRIO DIGITOU ARTISTA + MÚSICA (ex: "Daft Punk Instant Crush")
            # -----------------------------------------------------------------
            if " " in q:
                matched_artist_id = None
                matched_artist_name = None
                words = q.split(" ")
                
                # Testa se a primeira palavra ou primeiras 2 palavras batem com um artista conhecido
                for n_words in [2, 1]:
                    if len(words) > n_words:
                        candidate_art = " ".join(words[:n_words])
                        candidate_trk = " ".join(words[n_words:])
                        
                        art_row = conn.execute(
                            """
                            SELECT id, name FROM music_artists
                            WHERE LOWER(name) = ? OR LOWER(name) LIKE ? || '%'
                            ORDER BY CASE WHEN LOWER(name) = ? THEN 1 ELSE 2 END, listeners DESC
                            LIMIT 1
                            """,
                            [candidate_art.lower(), candidate_art.lower(), candidate_art.lower()]
                        ).fetchone()
                        
                        if art_row:
                            matched_artist_id, matched_artist_name = art_row
                            sql_art_trk = """
                            SELECT 
                                'track:' || CAST(t.id AS VARCHAR) AS node_id,
                                'track' AS entity_type,
                                t.title,
                                ma.name AS artist_name,
                                alb.title AS album_name,
                                COALESCE(t.popularity, 0) AS popularity,
                                t.tempo,
                                t.valence,
                                t.energy,
                                t.danceability,
                                t.acousticness,
                                t.instrumentalness,
                                ma.country
                            FROM music_tracks t
                            JOIN music_artists ma ON ma.id = t.artist_id
                            LEFT JOIN music_albums alb ON alb.id = t.album_id
                            WHERE t.artist_id = ?
                              AND strip_accents(LOWER(t.title)) LIKE '%' || strip_accents(?) || '%'
                            ORDER BY t.popularity DESC
                            LIMIT 3;
                            """
                            for row in conn.execute(sql_art_trk, [matched_artist_id, candidate_trk.lower()]).fetchall():
                                node_id = row[0]
                                if node_id not in seen_ids:
                                    seen_ids.add(node_id)
                                    results.append(ResolvedEntity(*row))
                            if len(results) >= limit:
                                break

            # -----------------------------------------------------------------
            # 3. BUSCA POR TÍTULO DA FAIXA (Usa idx_tracks_title)
            # -----------------------------------------------------------------
            if len(results) < limit:
                needed = limit - len(results)
                placeholders_trk = " OR ".join(["t.title = ? OR t.title LIKE ? || '%'" for _ in variants])
                params_trk = []
                for v in variants:
                    params_trk.extend([v, v])

                sql_tracks = f"""
                SELECT 
                    'track:' || CAST(t.id AS VARCHAR) AS node_id,
                    'track' AS entity_type,
                    t.title,
                    ma.name AS artist_name,
                    alb.title AS album_name,
                    COALESCE(t.popularity, 0) AS popularity,
                    t.tempo,
                    t.valence,
                    t.energy,
                    t.danceability,
                    t.acousticness,
                    t.instrumentalness,
                    ma.country
                FROM music_tracks t
                JOIN music_artists ma ON ma.id = t.artist_id
                LEFT JOIN music_albums alb ON alb.id = t.album_id
                WHERE {placeholders_trk}
                ORDER BY t.popularity DESC
                LIMIT ?;
                """
                for row in conn.execute(sql_tracks, params_trk + [needed * 2]).fetchall():
                    node_id = row[0]
                    if node_id not in seen_ids:
                        seen_ids.add(node_id)
                        results.append(ResolvedEntity(*row))
                    if len(results) >= limit:
                        break

            # -----------------------------------------------------------------
            # 4. FALLBACK: BUSCA POR STRIP_ACCENTS EM FAIXAS DE ALTA POPULARIDADE
            # -----------------------------------------------------------------
            if len(results) < limit:
                needed = limit - len(results)
                sql_fallback = """
                SELECT 
                    'track:' || CAST(t.id AS VARCHAR) AS node_id,
                    'track' AS entity_type,
                    t.title,
                    ma.name AS artist_name,
                    alb.title AS album_name,
                    COALESCE(t.popularity, 0) AS popularity,
                    t.tempo,
                    t.valence,
                    t.energy,
                    t.danceability,
                    t.acousticness,
                    t.instrumentalness,
                    ma.country
                FROM music_tracks t
                JOIN music_artists ma ON ma.id = t.artist_id
                LEFT JOIN music_albums alb ON alb.id = t.album_id
                WHERE t.popularity >= 30
                  AND strip_accents(LOWER(t.title)) LIKE '%' || strip_accents(?) || '%'
                ORDER BY t.popularity DESC
                LIMIT ?;
                """
                for row in conn.execute(sql_fallback, [q_lower, needed]).fetchall():
                    node_id = row[0]
                    if node_id not in seen_ids:
                        seen_ids.add(node_id)
                        results.append(ResolvedEntity(*row))
                    if len(results) >= limit:
                        break

        return results[:limit]
