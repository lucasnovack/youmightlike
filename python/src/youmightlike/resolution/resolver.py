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
            return f"[track] {self.title} — [bold cyan]{self.artist_name}[/bold cyan] (Pop: {self.popularity}){vibe_str}"
        else:
            loc = f" ({self.country})" if self.country else ""
            return f"[artist] [bold green]{self.title}[/bold green]{loc}"


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
            # 1. FAST-PATH: BUSCA EXATA POR ARTISTA (Usa idx_artists_name - ~40ms)
            # -----------------------------------------------------------------
            placeholders_exact_art = ", ".join(["?" for _ in variants])
            sql_exact_artist = f"""
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
            WHERE name IN ({placeholders_exact_art})
            ORDER BY 
                CASE WHEN LOWER(name) = ? THEN 1 ELSE 2 END,
                listeners DESC
            LIMIT 3;
            """
            for row in conn.execute(sql_exact_artist, variants + [q_lower]).fetchall():
                node_id = row[0]
                if node_id not in seen_ids:
                    seen_ids.add(node_id)
                    results.append(ResolvedEntity(*row))

            # Se já achou artista exato relevante e a query não tem espaço, podemos já retornar
            if len(results) >= limit:
                return results[:limit]

            # -----------------------------------------------------------------
            # 2. SE O USUÁRIO DIGITOU ARTISTA + MÚSICA (ex: "Daft Punk Instant Crush")
            # -----------------------------------------------------------------
            if " " in q:
                words = q.split(" ")
                for n_words in [2, 1]:
                    if len(words) > n_words and len(results) < limit:
                        candidate_art = " ".join(words[:n_words])
                        candidate_trk = " ".join(words[n_words:])
                        
                        art_vars = list(dict.fromkeys([candidate_art, candidate_art.title(), candidate_art.capitalize()]))
                        art_placeholders = ", ".join(["?" for _ in art_vars])
                        
                        art_row = conn.execute(
                            f"""
                            SELECT id, name FROM music_artists
                            WHERE name IN ({art_placeholders})
                            ORDER BY listeners DESC
                            LIMIT 1
                            """,
                            art_vars
                        ).fetchone()
                        
                        if art_row:
                            matched_artist_id, matched_artist_name = art_row
                            trk_vars = list(dict.fromkeys([candidate_trk, candidate_trk.title(), candidate_trk.capitalize()]))
                            trk_conds = " OR ".join(["t.title = ? OR t.title LIKE ? || '%'" for _ in trk_vars])
                            trk_params = []
                            for tv in trk_vars:
                                trk_params.extend([tv, tv])

                            sql_art_trk = f"""
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
                              AND ({trk_conds})
                            ORDER BY t.popularity DESC
                            LIMIT 3;
                            """
                            for row in conn.execute(sql_art_trk, [matched_artist_id] + trk_params).fetchall():
                                node_id = row[0]
                                if node_id not in seen_ids:
                                    seen_ids.add(node_id)
                                    results.append(ResolvedEntity(*row))
                            if len(results) >= limit:
                                break

            # -----------------------------------------------------------------
            # 3. BUSCA POR TÍTULO DA FAIXA (Fast-path: Exato + Prefixo via idx_tracks_title)
            # -----------------------------------------------------------------
            if len(results) < limit:
                needed = limit - len(results)
                # Fazemos primeiro busca de prefixo/exato sem ORDER BY na query para DuckDB usar o índice ART
                # e ordenamos os candidatos retornados em memória (~100-300ms vs 4000ms de scan)
                trk_conds = " OR ".join(["title = ? OR title LIKE ? || '%'" for _ in variants])
                trk_params = []
                for v in variants:
                    trk_params.extend([v, v])

                raw_matched = conn.execute(
                    f"""
                    SELECT id, title, artist_id, album_id, popularity,
                           tempo, valence, energy, danceability, acousticness, instrumentalness
                    FROM music_tracks
                    WHERE {trk_conds}
                    """,
                    trk_params
                ).fetchall()

                if raw_matched:
                    # Ordena em memória por popularidade
                    raw_matched.sort(key=lambda x: (x[4] or 0), reverse=True)
                    top_tracks = raw_matched[:needed * 2]

                    # Enriquecimento rápido em lote dos artistas e álbuns
                    art_ids = list({t[2] for t in top_tracks if t[2]})
                    alb_ids = list({t[3] for t in top_tracks if t[3]})

                    art_map = {}
                    if art_ids:
                        art_in = ", ".join(str(i) for i in art_ids)
                        for r_art in conn.execute(f"SELECT id, name, country FROM music_artists WHERE id IN ({art_in})").fetchall():
                            art_map[r_art[0]] = (r_art[1], r_art[2])

                    alb_map = {}
                    if alb_ids:
                        alb_in = ", ".join(str(i) for i in alb_ids)
                        for r_alb in conn.execute(f"SELECT id, title FROM music_albums WHERE id IN ({alb_in})").fetchall():
                            alb_map[r_alb[0]] = r_alb[1]

                    for t in top_tracks:
                        node_id = f"track:{t[0]}"
                        if node_id not in seen_ids:
                            seen_ids.add(node_id)
                            a_info = art_map.get(t[2], (None, None))
                            alb_name = alb_map.get(t[3])
                            results.append(ResolvedEntity(
                                node_id=node_id,
                                entity_type="track",
                                title=t[1],
                                artist_name=a_info[0],
                                album_name=alb_name,
                                popularity=t[4] or 0,
                                tempo=t[5],
                                valence=t[6],
                                energy=t[7],
                                danceability=t[8],
                                acousticness=t[9],
                                instrumentalness=t[10],
                                country=a_info[1]
                            ))
                        if len(results) >= limit:
                            break

            # -----------------------------------------------------------------
            # 4. PREFIX SEARCH EM ARTISTAS (caso nada tenha sido encontrado)
            # -----------------------------------------------------------------
            if not results:
                placeholders_prefix_art = " OR ".join(["name LIKE ? || '%'" for _ in variants])
                sql_prefix_artist = f"""
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
                WHERE {placeholders_prefix_art}
                ORDER BY listeners DESC
                LIMIT ?;
                """
                for row in conn.execute(sql_prefix_artist, variants + [limit]).fetchall():
                    node_id = row[0]
                    if node_id not in seen_ids:
                        seen_ids.add(node_id)
                        results.append(ResolvedEntity(*row))

        return results[:limit]
