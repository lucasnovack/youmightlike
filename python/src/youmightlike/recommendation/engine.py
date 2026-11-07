from __future__ import annotations

import json
import math
import subprocess
import urllib.request
from dataclasses import dataclass
from pathlib import Path

import duckdb

from youmightlike.resolution.resolver import ResolvedEntity

RUST_ENGINE_BIN = Path("target/release/youmightlike-engine")
RUST_SERVER_URL = "http://127.0.0.1:8000"


@dataclass
class RecommendationResult:
    track_id: int
    title: str
    artist_name: str
    album_name: str | None
    popularity: int
    affinity_score: float  # 0.0 a 100.0%
    tempo: float | None
    valence: float | None
    energy: float | None
    danceability: float | None
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
    """Motor determinístico de difusão em grafo cultural e alinhamento acústico via Rust."""

    def __init__(
        self,
        db_path: str = "data/processed/taste_graph.duckdb",
        server_url: str = RUST_SERVER_URL,
    ) -> None:
        self.db_path = db_path
        self.server_url = server_url.rstrip("/")

    def _get_connection(self) -> duckdb.DuckDBPyConnection:
        conn = duckdb.connect(self.db_path, read_only=True)
        conn.execute("PRAGMA threads=8;")
        return conn

    def _recommend_via_rust_service(
        self,
        seed_node_ids: list[str],
        limit: int,
        min_popularity: int,
        audio_weight: float,
    ) -> list[RecommendationResult] | None:
        """Tenta usar o serviço Rust (youmightlike-server) com grafo CSR já residente em RAM."""
        payload = json.dumps({
            "seeds": seed_node_ids,
            "limit": limit,
            "min_popularity": min_popularity,
            "audio_weight": audio_weight,
        }).encode("utf-8")

        req = urllib.request.Request(
            f"{self.server_url}/api/recommend/tracks",
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(req, timeout=5.0) as resp:
                data = json.loads(resp.read().decode("utf-8"))
                return [
                    RecommendationResult(
                        track_id=item["track_id"],
                        title=item["title"],
                        artist_name=item["artist_name"],
                        album_name=item.get("album_name"),
                        popularity=item["popularity"],
                        affinity_score=item["affinity_score"],
                        tempo=item.get("tempo"),
                        valence=item.get("valence"),
                        energy=item.get("energy"),
                        danceability=item.get("danceability"),
                        explanation=item["explanation"],
                    )
                    for item in data
                ]
        except Exception:
            return None

    def _diffuse_with_rust_cli(
        self,
        seed_keys: list[str],
        limit: int = 25,
    ) -> list[dict]:
        """Invoca o binário nativo em Rust (youmightlike-engine) para difusão via Rayon."""
        if not RUST_ENGINE_BIN.exists():
            raise RuntimeError(
                f"Binário Rust '{RUST_ENGINE_BIN}' não encontrado. Compile com: cargo build --release"
            )

        cmd = [
            str(RUST_ENGINE_BIN),
            "--seeds", ",".join(seed_keys),
            "--target-prefix", "artist:",
            "--limit", str(limit),
            "--db-path", self.db_path,
            "--query-edges", "SELECT source_id, target_id, base_weight, relation_type FROM graph_semantic_edges WHERE base_weight > 0.0",
            "--json",
        ]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(res.stdout)

    def recommend(
        self,
        seeds: list[ResolvedEntity],
        limit: int = 10,
        min_popularity: int = 30,
        audio_weight: float = 0.4,
    ) -> list[RecommendationResult]:
        """Gera recomendações musicais a partir de sementes usando exclusivamente o motor Rust."""
        if not seeds:
            return []

        # 1. Caminho rápido: se o serviço Rust (`youmightlike-server`) estiver ativo em RAM, usa-o direto
        seed_node_ids = [s.node_id for s in seeds]
        from_service = self._recommend_via_rust_service(
            seed_node_ids,
            limit=limit,
            min_popularity=min_popularity,
            audio_weight=audio_weight,
        )
        if from_service is not None:
            return from_service

        # 2. Caso contrário, executa a difusão PPR via binário CLI Rust (`youmightlike-engine`)
        seed_artist_ids = set()
        seed_track_ids = set()
        seed_audio_features = []

        with self._get_connection() as conn:
            for s in seeds:
                if s.entity_type == "artist":
                    a_id = int(s.node_id.split(":")[1])
                    seed_artist_ids.add(a_id)
                elif s.entity_type == "track":
                    t_id = int(s.node_id.split(":")[1])
                    seed_track_ids.add(t_id)
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

            avg_audio = None
            if seed_audio_features:
                avg_audio = {
                    "tempo": sum(f["tempo"] for f in seed_audio_features) / len(seed_audio_features),
                    "valence": sum(f["valence"] for f in seed_audio_features) / len(seed_audio_features),
                    "energy": sum(f["energy"] for f in seed_audio_features) / len(seed_audio_features),
                    "danceability": sum(f["danceability"] for f in seed_audio_features) / len(seed_audio_features),
                }

            seed_keys = [f"artist:{aid}" for aid in seed_artist_ids]
            rust_results = self._diffuse_with_rust_cli(seed_keys, limit=25)
            if not rust_results:
                return []

            scored_map = {
                int(r["node_key"].split(":")[1]): r["score"]
                for r in rust_results
                if ":" in r["node_key"]
            }
            art_ids_str = ", ".join(str(aid) for aid in scored_map)
            if not art_ids_str:
                return []

            sql_candidates = f"""
            WITH target_artists AS (
                SELECT id, name FROM music_artists WHERE id IN ({art_ids_str})
            ),
            artist_reasons AS (
                SELECT 
                    s.target_artist_id AS artist_id,
                    'Co-escuta Last.fm com ' || ma_src.name || ' (' || CAST(ROUND(s.match_score * 100) AS INT) || '%)' AS reason
                FROM artist_similarities s
                JOIN music_artists ma_src ON ma_src.id = s.source_artist_id
                WHERE s.source_artist_id IN ({", ".join(str(i) for i in seed_artist_ids)})
                  AND s.target_artist_id IN ({art_ids_str})
                UNION ALL
                SELECT 
                    t2.artist_id,
                    'Microgêneros com ' || ma_src.name || ': ' || STRING_AGG(k.name, ', ') AS reason
                FROM artist_tags t1
                JOIN music_artists ma_src ON ma_src.id = t1.artist_id
                JOIN keywords k ON k.id = t1.keyword_id
                JOIN artist_tags t2 ON t2.keyword_id = k.id AND t2.artist_id IN ({art_ids_str})
                WHERE t1.artist_id IN ({", ".join(str(i) for i in seed_artist_ids)})
                GROUP BY t2.artist_id, ma_src.name
            ),
            aggregated_reasons AS (
                SELECT artist_id, STRING_AGG(reason, ' • ') AS full_explanation
                FROM artist_reasons
                GROUP BY artist_id
            )
            SELECT 
                t.id AS track_id,
                t.title,
                ta.name AS artist_name,
                alb.title AS album_name,
                t.popularity,
                t.tempo,
                t.valence,
                t.energy,
                t.danceability,
                t.artist_id,
                COALESCE(ar.full_explanation, 'Conectado pela topologia estocástica do grafo cultural') AS explanation
            FROM target_artists ta
            JOIN music_tracks t ON t.artist_id = ta.id
            LEFT JOIN music_albums alb ON alb.id = t.album_id
            LEFT JOIN aggregated_reasons ar ON ar.artist_id = ta.id
            WHERE t.popularity >= {min_popularity}
            ORDER BY t.popularity DESC
            LIMIT 150;
            """
            raw_candidates = conn.execute(sql_candidates).fetchall()
            candidates = []
            for rc in raw_candidates:
                aid = rc[9]
                rust_score = scored_map.get(aid, 0.001)
                candidates.append((rc[0], rc[1], rc[2], rc[3], rc[4], rc[5], rc[6], rc[7], rc[8], rust_score, rc[10]))

            if not candidates:
                return []

            max_graph_score = max(c[9] for c in candidates) if candidates else 1.0

            recommendations: list[RecommendationResult] = []
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

                audio_affinity = 0.8
                if avg_audio and c[5] is not None and c[6] is not None and c[7] is not None:
                    c_tempo = c[5]
                    c_valence = c[6]
                    c_energy = c[7]
                    c_dance = c[8] or 0.5

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
