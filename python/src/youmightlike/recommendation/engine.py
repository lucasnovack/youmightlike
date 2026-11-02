from __future__ import annotations

import math
from dataclasses import dataclass
from typing import List, Optional
import duckdb

from youmightlike.resolution.resolver import ResolvedEntity


@dataclass
class RecommendationResult:
    track_id: int
    title: str
    artist_name: str
    album_name: Optional[str]
    popularity: int
    affinity_score: float  # 0.0 a 100.0%
    tempo: Optional[float]
    valence: Optional[float]
    energy: Optional[float]
    danceability: Optional[float]
    explanation: str

    def vibe_summary(self) -> str:
        parts = []
        if self.tempo:
            parts.append(f"{int(round(self.tempo))} BPM")
        if self.valence is not None:
            parts.append("Upbeat" if self.valence >= 0.5 else "Melancólico")
        if self.energy is not None:
            parts.append(f"Energia {int(self.energy * 100)}%")
        return " • ".join(parts) if parts else "N/A"


class MusicRecommenderEngine:
    """Motor determinístico de difusão em grafo cultural e alinhamento acústico."""

    def __init__(self, db_path: str = "data/processed/taste_graph.duckdb") -> None:
        self.db_path = db_path

    def _get_connection(self) -> duckdb.DuckDBPyConnection:
        conn = duckdb.connect(self.db_path, read_only=True)
        conn.execute("PRAGMA threads=8;")
        return conn

    def recommend(
        self,
        seeds: List[ResolvedEntity],
        limit: int = 10,
        min_popularity: int = 30,
        audio_weight: float = 0.4,
    ) -> List[RecommendationResult]:
        """Gera recomendações musicais a partir de uma ou mais sementes com justificativa de rota."""
        if not seeds:
            return []

        # 1. Isolar IDs de artistas das sementes
        seed_artist_ids = set()
        seed_track_ids = set()
        seed_audio_features = []

        with self._get_connection() as conn:
            for s in seeds:
                if s.entity_type == "artist":
                    # Extrai ID numérico do nó 'artist:123'
                    a_id = int(s.node_id.split(":")[1])
                    seed_artist_ids.add(a_id)
                elif s.entity_type == "track":
                    t_id = int(s.node_id.split(":")[1])
                    seed_track_ids.add(t_id)
                    # Busca artist_id da faixa se não tiver
                    art_row = conn.execute(
                        "SELECT artist_id, tempo, valence, energy, danceability FROM music_tracks WHERE id = ?",
                        [t_id],
                    ).fetchone()
                    if art_row:
                        seed_artist_ids.add(art_row[0])
                        if art_row[1] is not None:
                            seed_audio_features.append({
                                "tempo": art_row[1],
                                "valence": art_row[2] or 0.5,
                                "energy": art_row[3] or 0.5,
                                "danceability": art_row[4] or 0.5,
                            })

            if not seed_artist_ids:
                return []

            # Perfil acústico médio das sementes
            avg_audio = None
            if seed_audio_features:
                avg_audio = {
                    "tempo": sum(f["tempo"] for f in seed_audio_features) / len(seed_audio_features),
                    "valence": sum(f["valence"] for f in seed_audio_features) / len(seed_audio_features),
                    "energy": sum(f["energy"] for f in seed_audio_features) / len(seed_audio_features),
                    "danceability": sum(f["danceability"] for f in seed_audio_features) / len(seed_audio_features),
                }

            artist_list_sql = ", ".join(str(i) for i in seed_artist_ids)

            # -----------------------------------------------------------------
            # 2. DIFUSÃO NO GRAFO ESTRUTURAL (Last.fm Co-listening + Tags Adamic-Adar)
            # -----------------------------------------------------------------
            sql_diffusion = f"""
            WITH direct_similars AS (
                -- Relações diretas Last.fm (co-listening ponderado)
                SELECT 
                    s.target_artist_id AS artist_id,
                    s.match_score * 4.0 AS score,
                    'Co-escuta com ' || ma_src.name || ' (' || CAST(ROUND(s.match_score * 100) AS INT) || '% Last.fm)' AS reason
                FROM artist_similarities s
                JOIN music_artists ma_src ON ma_src.id = s.source_artist_id
                WHERE s.source_artist_id IN ({artist_list_sql})
                  AND s.target_artist_id NOT IN ({artist_list_sql})
            ),
            shared_tags AS (
                -- Microgêneros e tags estéticas compartilhadas com IDF/Adamic-Adar
                SELECT 
                    t2.artist_id,
                    SUM((t1.weight * t2.weight) / ln(1.0 + COALESCE(k_deg.deg, 10.0))) * 1.5 AS score,
                    'Microgêneros com ' || ma_src.name || ': ' || STRING_AGG(k.name, ', ') AS reason
                FROM artist_tags t1
                JOIN music_artists ma_src ON ma_src.id = t1.artist_id
                JOIN keywords k ON k.id = t1.keyword_id
                LEFT JOIN (
                    SELECT keyword_id, COUNT(*) AS deg FROM artist_tags GROUP BY keyword_id
                ) k_deg ON k_deg.keyword_id = k.id
                JOIN artist_tags t2 ON t2.keyword_id = k.id AND t2.artist_id NOT IN ({artist_list_sql})
                WHERE t1.artist_id IN ({artist_list_sql})
                GROUP BY t2.artist_id, ma_src.name
                HAVING COUNT(k.id) >= 2
                ORDER BY score DESC
                LIMIT 50
            ),
            all_candidate_artists AS (
                SELECT artist_id, score, reason FROM direct_similars
                UNION ALL
                SELECT artist_id, score, reason FROM shared_tags
            ),
            aggregated_artists AS (
                SELECT 
                    artist_id,
                    SUM(score) AS total_graph_score,
                    STRING_AGG(reason, ' • ') AS full_explanation
                FROM all_candidate_artists
                GROUP BY artist_id
                ORDER BY total_graph_score DESC
                LIMIT 25
            )
            -- Busca as faixas mais aclamadas dos artistas selecionados pelo grafo
            SELECT 
                t.id AS track_id,
                t.title,
                ma.name AS artist_name,
                alb.title AS album_name,
                t.popularity,
                t.tempo,
                t.valence,
                t.energy,
                t.danceability,
                aa.total_graph_score,
                aa.full_explanation
            FROM aggregated_artists aa
            JOIN music_artists ma ON ma.id = aa.artist_id
            JOIN music_tracks t ON t.artist_id = aa.artist_id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            WHERE t.popularity >= {min_popularity}
            ORDER BY (aa.total_graph_score * 5.0 + t.popularity) DESC
            LIMIT 150;
            """

            candidates = conn.execute(sql_diffusion).fetchall()
            if not candidates:
                return []

            # -----------------------------------------------------------------
            # 3. RERANKING COM PROXIMIDADE ACÚSTICA CONTÍNUA E DIVERSIDADE
            # -----------------------------------------------------------------
            max_graph_score = max(c[9] for c in candidates) if candidates else 1.0

            recommendations: List[RecommendationResult] = []
            seen_tracks = set()
            seen_artist_titles = set()
            artist_counts: dict[str, int] = {}
            max_per_artist = 2

            for c in candidates:
                t_id = c[0]
                t_title = c[1].lower().strip()
                art_name = c[2]
                if t_id in seed_track_ids or t_id in seen_tracks:
                    continue
                if (art_name, t_title) in seen_artist_titles:
                    continue
                if artist_counts.get(art_name, 0) >= max_per_artist:
                    continue

                seen_tracks.add(t_id)
                seen_artist_titles.add((art_name, t_title))
                artist_counts[art_name] = artist_counts.get(art_name, 0) + 1

                graph_norm = min(1.0, c[9] / max_graph_score)

                # Cálculo de distância acústica
                audio_affinity = 0.8  # Padrão se não houver atributos
                if avg_audio and c[5] is not None and c[6] is not None and c[7] is not None:
                    c_tempo = c[5]
                    c_valence = c[6]
                    c_energy = c[7]
                    c_dance = c[8] or 0.5

                    # Distância euclidiana ponderada
                    d_valence = (c_valence - avg_audio["valence"]) ** 2
                    d_energy = (c_energy - avg_audio["energy"]) ** 2
                    d_dance = (c_dance - avg_audio["danceability"]) ** 2
                    d_tempo = ((c_tempo - avg_audio["tempo"]) / 120.0) ** 2

                    dist = math.sqrt(0.3 * d_valence + 0.3 * d_energy + 0.2 * d_dance + 0.2 * d_tempo)
                    audio_affinity = max(0.0, 1.0 - dist)

                final_affinity = (1.0 - audio_weight) * graph_norm + audio_weight * audio_affinity
                affinity_percentage = round(final_affinity * 100.0, 1)

                recommendations.append(
                    RecommendationResult(
                        track_id=t_id,
                        title=c[1],
                        artist_name=c[2],
                        album_name=c[3],
                        popularity=c[4],
                        affinity_score=affinity_percentage,
                        tempo=c[5],
                        valence=c[6],
                        energy=c[7],
                        danceability=c[8],
                        explanation=c[10],
                    )
                )

            recommendations.sort(key=lambda r: r.affinity_score, reverse=True)
            return recommendations[:limit]
