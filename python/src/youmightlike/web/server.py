"""FastAPI server and REST API for the youmightlike UI."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any, Dict, List, Optional

import duckdb
from fastapi import FastAPI, Query
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from youmightlike.resolution.resolver import MusicResolver

app = FastAPI(
    title="youmightlike",
    description="Transmedia cultural discovery engine and taste graph",
    version="0.3.0",
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

DB_PATH = "data/processed/taste_graph.duckdb"
RUST_ENGINE_BIN = Path("target/release/youmightlike-engine")
STATIC_DIR = Path(__file__).parent / "static"


def get_db():
    conn = duckdb.connect(DB_PATH, read_only=True)
    conn.execute("PRAGMA threads=8;")
    return conn


# ---------------------------------------------------------------------------
# Models
# ---------------------------------------------------------------------------
class EntityItem(BaseModel):
    id: str  # ex: 'artist:11', 'movie:12406', 'game:442', 'track:60496'
    name: str
    type: str  # 'artist' | 'track' | 'movie' | 'game' | 'book'
    subtitle: Optional[str] = None
    extra: Optional[Dict[str, Any]] = None


class RecommendationRequest(BaseModel):
    seeds: List[str]  # Lista de IDs: ['artist:11', 'movie:12406']
    target_type: Optional[str] = None  # None ou 'music', 'movie', 'game', 'book'
    limit: int = 12
    lang: str = "pt"  # 'pt' | 'en'



class GraphNode(BaseModel):
    id: str
    label: str
    type: str  # 'seed' | 'artist' | 'track' | 'movie' | 'game' | 'book' | 'tag' | 'person'
    val: float  # tamanho do nó


class GraphEdge(BaseModel):
    source: str
    target: str
    label: Optional[str] = None
    weight: float = 1.0


class RecommendationResponse(BaseModel):
    seeds: List[EntityItem]
    results: List[Dict[str, Any]]
    graph: Dict[str, Any]  # {'nodes': [...], 'links': [...]}


# ---------------------------------------------------------------------------
# Endpoints
# ---------------------------------------------------------------------------
@app.get("/api/search")
def search_entities(
    q: str = Query(..., min_length=2),
    entity_type: Optional[str] = Query(None),  # 'artist', 'track', 'movie', 'game'
    limit: int = 10,
) -> List[EntityItem]:
    """Busca em tempo real com filtro opcional de tipo e matching flexível."""
    q_str = q.strip()
    if not q_str:
        return []

    items: List[EntityItem] = []
    seen = set()

    with get_db() as conn:
        # 1. Artistas (se entity_type for None ou 'artist')
        if not entity_type or entity_type == "artist":
            variants = list(dict.fromkeys([q_str, q_str.title(), q_str.capitalize()]))
            art_placeholders = ", ".join(["?" for _ in variants])
            art_rows = conn.execute(
                f"""
                SELECT ma.id, ma.name, ma.country, ma.listeners,
                       (SELECT k.name FROM artist_tags tag_rel JOIN keywords k ON k.id = tag_rel.keyword_id WHERE tag_rel.artist_id = ma.id ORDER BY tag_rel.weight DESC LIMIT 1) as top_tag
                FROM music_artists ma
                WHERE ma.name IN ({art_placeholders}) 
                   OR ma.name ILIKE ? || '%'
                   OR ma.name ILIKE '%' || ? || '%'
                ORDER BY 
                    CASE WHEN LOWER(ma.name) = LOWER(?) || ' (br)' THEN 1
                         WHEN LOWER(ma.name) = LOWER(?) THEN 2
                         WHEN LOWER(ma.name) LIKE LOWER(?) || ' (%)' THEN 3
                         WHEN LOWER(ma.name) LIKE LOWER(?) || '%' THEN 4
                         WHEN ma.name NOT LIKE '%;%' THEN 5
                         ELSE 6 END,
                    CASE WHEN ma.country = 'BR' THEN 1 WHEN ma.country IS NOT NULL THEN 2 ELSE 3 END,
                    LENGTH(ma.name) ASC,
                    ma.listeners DESC 
                LIMIT 8;
                """,
                variants + [q_str, q_str, q_str, q_str, q_str, q_str],
            ).fetchall()

            for aid, aname, country, listeners, top_tag in art_rows:
                nid = f"artist:{aid}"
                if nid not in seen:
                    seen.add(nid)
                    genre_str = f" • #{top_tag}" if top_tag else ""
                    loc_str = f" ({country})" if country else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=aname,
                            type="artist",
                            subtitle=f"Artist{loc_str}{genre_str}",
                        )
                    )

        # 2. Jogos (se entity_type for None ou 'game')
        if not entity_type or entity_type == "game":
            game_rows = conn.execute(
                """
                SELECT id, title, release_year, positive_reviews 
                FROM games 
                WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
                ORDER BY 
                    CASE WHEN LOWER(title) = LOWER(?) THEN 1 
                         WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2 
                         ELSE 3 END,
                    positive_reviews DESC NULLS LAST
                LIMIT 6;
                """,
                [q_str, q_str, q_str, q_str, q_str],
            ).fetchall()

            for gid, gtitle, gyear, grevs in game_rows:
                nid = f"game:{gid}"
                if nid not in seen:
                    seen.add(nid)
                    year_str = f" ({gyear})" if gyear else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=gtitle,
                            type="game",
                            subtitle=f"Game{year_str}",
                        )
                    )

        # 3. Filmes (se entity_type for None ou 'movie')
        if not entity_type or entity_type == "movie":
            mov_rows = conn.execute(
                """
                SELECT id, title, release_year, vote_average 
                FROM movies 
                WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
                ORDER BY 
                    CASE WHEN LOWER(title) = LOWER(?) THEN 1 
                         WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2 
                         ELSE 3 END,
                    vote_count DESC NULLS LAST
                LIMIT 6;
                """,
                [q_str, q_str, q_str, q_str, q_str],
            ).fetchall()

            for mid, mtitle, myear, mvote in mov_rows:
                nid = f"movie:{mid}"
                if nid not in seen:
                    seen.add(nid)
                    year_str = f" ({myear})" if myear else ""
                    score_str = f" • Rating: {mvote}" if mvote else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=mtitle,
                            type="movie",
                            subtitle=f"Movie{year_str}{score_str}",
                        )
                    )

        # 3.1 Séries de TV (se entity_type for None ou 'series')
        if not entity_type or entity_type == "series":
            series_rows = conn.execute(
                """
                SELECT id, title, start_year, end_year, vote_average, series_type
                FROM tv_series 
                WHERE title = ? OR title ILIKE ? || '%' OR title ILIKE '%' || ? || '%'
                ORDER BY 
                    CASE WHEN LOWER(title) = LOWER(?) THEN 1 
                         WHEN LOWER(title) LIKE LOWER(?) || '%' THEN 2 
                         ELSE 3 END,
                    vote_count DESC NULLS LAST
                LIMIT 6;
                """,
                [q_str, q_str, q_str, q_str, q_str],
            ).fetchall()

            for sid, stitle, sstart, send, svote, stype in series_rows:
                nid = f"series:{sid}"
                if nid not in seen:
                    seen.add(nid)
                    years_str = f" ({sstart}-{send})" if (sstart and send and sstart != send) else (f" ({sstart})" if sstart else "")
                    type_label = "Minissérie" if stype == "tvMiniSeries" else "Série"
                    score_str = f" • Rating: {svote:.1f}" if svote else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=stitle,
                            type="series",
                            subtitle=f"{type_label}{years_str}{score_str}",
                        )
                    )

        # 4. Músicas mais populares com esse título (se entity_type for None ou 'track')
        if not entity_type or entity_type == "track":
            needed = (limit - len(items)) * 3
            trk_rows = conn.execute(
                """
                SELECT t.id, t.title, ma.name, t.popularity, t.tempo, t.valence, t.energy, alb.title
                FROM music_tracks t
                JOIN music_artists ma ON ma.id = t.artist_id
                LEFT JOIN music_albums alb ON alb.id = t.album_id
                WHERE t.title = ? OR t.title ILIKE ? || '%' OR t.title ILIKE '%' || ? || '%'
                ORDER BY 
                    CASE WHEN LOWER(t.title) = LOWER(?) THEN 1 
                         WHEN LOWER(t.title) LIKE LOWER(?) || '%' THEN 2 
                         ELSE 3 END,
                    t.popularity DESC
                LIMIT ?;
                """,
                [q_str, q_str, q_str, q_str, q_str, needed if needed > 5 else 10],
            ).fetchall()

            seen_track_keys = set()
            for tid, ttitle, taname, tpop, bpm, val, eng, albtitle in trk_rows:
                key = (ttitle.lower().strip(), taname.lower().strip())
                if key in seen_track_keys:
                    continue
                seen_track_keys.add(key)
                nid = f"track:{tid}"
                if nid not in seen:
                    seen.add(nid)
                    alb_str = f" • {albtitle}" if albtitle else ""
                    vibe = f" ({int(round(bpm))} BPM)" if bpm else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=ttitle,
                            type="track",
                            subtitle=f"{taname}{alb_str}{vibe}",
                            extra={"popularity": tpop, "artist_name": taname},
                        )
                    )
                if len(items) >= limit:
                    break

    return items[:limit]


# Cache em memória para evitar recalculação redundante em cliques rápidos
_RECOMMENDATION_CACHE: Dict[str, RecommendationResponse] = {}
_MAX_CACHE_SIZE = 128


@app.post("/api/recommend", response_model=RecommendationResponse)
def get_recommendations(req: RecommendationRequest) -> RecommendationResponse:
    """Gera recomendações a partir do perfil e monta o subgrafo interativo."""
    if not req.seeds:
        return RecommendationResponse(seeds=[], results=[], graph={"nodes": [], "links": []})

    # Cache key determinística ordenada
    cache_key = f"{','.join(sorted(req.seeds))}|{req.target_type or 'all'}|{req.limit}|{req.lang}"
    if cache_key in _RECOMMENDATION_CACHE:
        return _RECOMMENDATION_CACHE[cache_key]


    seed_set = set(req.seeds)
    seeds_resolved: List[EntityItem] = []
    diffusion_seeds: List[str] = []

    with get_db() as conn:
        # 1. Carrega os metadados dos nós semente
        for s in req.seeds:
            prefix, s_id = s.split(":")
            nid = int(s_id)
            if prefix == "artist":

                row = conn.execute("SELECT name, country FROM music_artists WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="artist", subtitle=row[1]))
                    diffusion_seeds.append(s)
            elif prefix == "movie":
                row = conn.execute("SELECT title, release_year FROM movies WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="movie", subtitle=str(row[1])))
                    diffusion_seeds.append(s)
            elif prefix == "series":
                row = conn.execute("SELECT title, start_year FROM tv_series WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="series", subtitle=str(row[1])))
                    diffusion_seeds.append(s)
            elif prefix == "game":
                row = conn.execute("SELECT title, release_year FROM games WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="game", subtitle=str(row[1])))
                    diffusion_seeds.append(s)
            elif prefix == "track":
                row = conn.execute(
                    "SELECT t.title, ma.name, t.artist_id FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?",
                    [nid],
                ).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="track", subtitle=row[1]))
                    # Faixas conectam ao grafo semântico através do seu artista
                    art_node = f"artist:{row[2]}"
                    if art_node not in diffusion_seeds:
                        diffusion_seeds.append(art_node)

        # 2. Executa a difusão no motor Rust
        target_prefix = ""
        if req.target_type == "music":
            target_prefix = "artist:"
        elif req.target_type == "movie":
            target_prefix = "movie:"
        elif req.target_type == "series":
            target_prefix = "series:"
        elif req.target_type == "game":
            target_prefix = "game:"
        elif req.target_type == "book":
            target_prefix = "book:"

        cmd = [
            str(RUST_ENGINE_BIN),
            "--seeds",
            ",".join(diffusion_seeds if diffusion_seeds else req.seeds),
            "--limit",
            str(max(req.limit * 8, 150)),
            "--db-path",
            DB_PATH,
            "--query-edges",
            "SELECT source_id, target_id, base_weight FROM graph_semantic_edges",
            "--json",
        ]
        if target_prefix:
            cmd.extend(["--target-prefix", target_prefix])

        diffusion_results = []
        try:
            res = subprocess.run(cmd, capture_output=True, text=True, check=True)
            diffusion_results = json.loads(res.stdout)
        except Exception:
            diffusion_results = []

        # 3. Enriquece os resultados
        results: List[Dict[str, Any]] = []
        graph_nodes: List[Dict[str, Any]] = []
        graph_links: List[Dict[str, Any]] = []

        # Adiciona os nós semente ao grafo
        for seed_item in seeds_resolved:
            graph_nodes.append({
                "id": seed_item.id,
                "label": seed_item.name,
                "type": "seed",
                "val": 18.0,
            })
            # Se for faixa, adiciona o artista correspondente e a aresta direta
            if seed_item.type == "track":
                t_id = int(seed_item.id.split(":")[1])
                t_art = conn.execute("SELECT ma.id, ma.name FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?", [t_id]).fetchone()
                if t_art:
                    art_key = f"artist:{t_art[0]}"
                    if not any(n["id"] == art_key for n in graph_nodes):
                        graph_nodes.append({
                            "id": art_key,
                            "label": t_art[1],
                            "type": "artist",
                            "val": 14.0,
                        })
                    graph_links.append({
                        "source": seed_item.id,
                        "target": art_key,
                        "label": "performed_by",
                        "weight": 2.5,
                    })

            # Garante nós âncora estruturais para cada semente (ex: Developer para games, Tags para artistas)
            seed_pfx = seed_item.id.split(":")[0]
            anchor_rows = conn.execute(
                """
                SELECT target_id, relation_type, base_weight
                FROM graph_semantic_edges
                WHERE source_id = ?
                ORDER BY 
                    CASE WHEN relation_type = 'developed_by' THEN 1
                         WHEN relation_type = 'has_director' THEN 2
                         WHEN relation_type = 'has_tag' THEN 3
                         ELSE 4 END,
                    base_weight DESC
                LIMIT 2;
                """,
                [seed_item.id],
            ).fetchall()

            for target_id, rel, w in anchor_rows:
                t_pfx, t_raw = target_id.split(":")
                t_nid = int(t_raw)
                t_label = ""
                if t_pfx == "developer":
                    drow = conn.execute("SELECT name FROM game_developers WHERE id = ?", [t_nid]).fetchone()
                    if drow:
                        t_label = drow[0]
                elif t_pfx == "keyword":
                    krow = conn.execute("SELECT name FROM keywords WHERE id = ?", [t_nid]).fetchone()
                    if krow:
                        t_label = f"#{krow[0]}"
                elif t_pfx == "person":
                    prow = conn.execute("SELECT name FROM people WHERE id = ?", [t_nid]).fetchone()
                    if prow:
                        t_label = prow[0]

                if t_label and not any(n["id"] == target_id for n in graph_nodes):
                    graph_nodes.append({
                        "id": target_id,
                        "label": t_label,
                        "type": t_pfx,
                        "val": 9.0,
                    })
                    graph_links.append({
                        "source": seed_item.id,
                        "target": target_id,
                        "label": rel,
                        "weight": w,
                    })

        # Adiciona explicitamente as tags ou atributos compartilhados diretamente entre as sementes
        if len(diffusion_seeds) > 1:
            seeds_in_sql = ", ".join(f"'{s}'" for s in diffusion_seeds)
            seed_bridges = conn.execute(
                f"""
                SELECT e1.target_id, k.name, count(*) as common_cnt
                FROM graph_semantic_edges e1
                JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                JOIN keywords k ON 'keyword:' || k.id = e1.target_id
                WHERE e1.source_id IN ({seeds_in_sql})
                  AND e2.source_id IN ({seeds_in_sql})
                  AND e1.source_id != e2.source_id
                  AND e1.relation_type = 'has_tag'
                  AND e2.relation_type = 'has_tag'
                GROUP BY e1.target_id, k.name
                ORDER BY common_cnt DESC
                LIMIT 3;
                """
            ).fetchall()
            for b_id, b_name, _ in seed_bridges:
                if not any(n["id"] == b_id for n in graph_nodes):
                    graph_nodes.append({
                        "id": b_id,
                        "label": f"#{b_name}",
                        "type": "keyword",
                        "val": 9.0,
                    })

        recs_count = 0
        bridge_count = 0
        seed_ids_sql = ", ".join(f"'{s}'" for s in (diffusion_seeds if diffusion_seeds else req.seeds))

        for item in diffusion_results:
            nkey = item["node_key"]
            score = item["score"]
            if nkey in seed_set:
                continue

            prefix, raw_id = nkey.split(":")
            nid = int(raw_id)


            if prefix == "artist" and recs_count < req.limit:
                row = conn.execute("SELECT name, country FROM music_artists WHERE id = ?", [nid]).fetchone()
                if row:
                    # Busca 1 faixa mais famosa desse artista
                    top_trk = conn.execute(
                        "SELECT title, popularity, tempo, valence, energy FROM music_tracks WHERE artist_id = ? ORDER BY popularity DESC LIMIT 1",
                        [nid],
                    ).fetchone()
                    trk_info = None
                    if top_trk:
                        trk_info = {
                            "title": top_trk[0],
                            "popularity": top_trk[1],
                            "tempo": top_trk[2],
                            "valence": top_trk[3],
                            "energy": top_trk[4],
                        }

                    # Explicação auditável: arestas diretas ou tags compartilhadas com as sementes
                    explanation_parts = []
                    seed_ids_sql = ", ".join(f"'{s}'" for s in (diffusion_seeds if diffusion_seeds else req.seeds))
                    
                    direct_edge = conn.execute(
                        f"""
                        SELECT e.relation_type, e.base_weight, ma_other.name
                        FROM graph_semantic_edges e
                        JOIN music_artists ma_other ON e.target_id = 'artist:' || ma_other.id OR e.source_id = 'artist:' || ma_other.id
                        WHERE ((e.source_id IN ({seed_ids_sql}) AND e.target_id = '{nkey}')
                           OR (e.source_id = '{nkey}' AND e.target_id IN ({seed_ids_sql})))
                          AND 'artist:' || ma_other.id != '{nkey}'
                        LIMIT 1;
                        """
                    ).fetchone()
                    is_pt = (req.lang == "pt")
                    if direct_edge:
                        rel, w, src_name = direct_edge
                        if rel == "similar_to":
                            pct = int(round(w * 100))
                            explanation_parts.append(f"Co-escuta com {src_name} ({pct}%)" if is_pt else f"Co-listening with {src_name} ({pct}%)")
                        else:
                            explanation_parts.append(f"Conexão direta com {src_name}" if is_pt else f"Direct link with {src_name}")

                    shared_tags = conn.execute(
                        f"""
                        SELECT k.name
                        FROM graph_semantic_edges e1
                        JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                        JOIN keywords k ON 'keyword:' || k.id = e1.target_id
                        WHERE e1.source_id IN ({seed_ids_sql})
                          AND e2.source_id = '{nkey}'
                          AND e1.relation_type = 'has_tag'
                          AND e2.relation_type = 'has_tag'
                        LIMIT 2;
                        """
                    ).fetchall()
                    if shared_tags:
                        tag_names = ", ".join(f"#{t[0]}" for t in shared_tags)
                        explanation_parts.append(f"Tags: {tag_names}")

                    default_artist_reason = "Difusão estocástica topológica" if is_pt else "Topological graph diffusion"
                    full_reason = " • ".join(explanation_parts) if explanation_parts else default_artist_reason
                    artist_subtitle = f"Artista Musical ({row[1] or 'Global'})" if is_pt else f"Musical Artist ({row[1] or 'Global'})"

                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "artist",
                        "subtitle": artist_subtitle,
                        "score": round(score * 1000, 2),
                        "top_track": trk_info,
                        "reason": full_reason,
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "artist",
                        "val": 12.0,
                    })
                    recs_count += 1

            elif prefix == "movie" and recs_count < req.limit:
                row = conn.execute("SELECT title, release_year, vote_average FROM movies WHERE id = ?", [nid]).fetchone()
                if row:
                    is_pt = (req.lang == "pt")
                    movie_explanation = []
                    # 1. Diretor ou estúdio compartilhado
                    shared_person = conn.execute(
                        f"""
                        SELECT p.name, e1.relation_type
                        FROM graph_semantic_edges e1
                        JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                        JOIN people p ON 'person:' || p.id = e1.target_id
                        WHERE e1.source_id IN ({seed_ids_sql})
                          AND e2.source_id = '{nkey}'
                          AND e1.relation_type IN ('has_director', 'has_composer')
                        LIMIT 1;
                        """
                    ).fetchone()
                    if shared_person:
                        movie_explanation.append(f"Mesmo realizador/compositor ({shared_person[0]})" if is_pt else f"Same director/composer ({shared_person[0]})")

                    # 2. Tags estéticas compartilhadas
                    shared_movie_tags = conn.execute(
                        f"""
                        SELECT k.name
                        FROM graph_semantic_edges e1
                        JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                        JOIN keywords k ON 'keyword:' || k.id = e1.target_id
                        WHERE e1.source_id IN ({seed_ids_sql})
                          AND e2.source_id = '{nkey}'
                          AND e1.relation_type = 'has_tag'
                          AND e2.relation_type = 'has_tag'
                        LIMIT 3;
                        """
                    ).fetchall()
                    if shared_movie_tags:
                        tag_str = ", ".join(f"#{t[0]}" for t in shared_movie_tags)
                        movie_explanation.append(f"Tags em comum: {tag_str}" if is_pt else f"Shared tags: {tag_str}")

                    default_movie_reason = "Afinidade de difusão de gênero e estilo" if is_pt else "Genre and style diffusion affinity"
                    full_movie_reason = " • ".join(movie_explanation) if movie_explanation else default_movie_reason
                    movie_subtitle = f"Filme ({row[1] or 'N/A'}) • Nota {row[2] or 'N/A'}" if is_pt else f"Movie ({row[1] or 'N/A'}) • Rating: {row[2] or 'N/A'}"

                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "movie",
                        "subtitle": movie_subtitle,
                        "score": round(score * 1000, 2),
                        "reason": full_movie_reason,
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "movie",
                        "val": 12.0,
                    })
                    recs_count += 1

            elif prefix == "game" and recs_count < req.limit:
                row = conn.execute("SELECT title, release_year FROM games WHERE id = ?", [nid]).fetchone()
                if row:
                    is_pt = (req.lang == "pt")
                    game_explanation = []
                    # 0. Co-jogabilidade direta da Steam (similar_to)
                    direct_game_edge = conn.execute(
                        f"""
                        SELECT e.relation_type, e.base_weight, g_other.title
                        FROM graph_semantic_edges e
                        JOIN games g_other ON (e.source_id = 'game:' || g_other.id AND e.target_id = '{nkey}')
                                           OR (e.target_id = 'game:' || g_other.id AND e.source_id = '{nkey}')
                        WHERE ((e.source_id IN ({seed_ids_sql}) AND e.target_id = '{nkey}')
                           OR (e.source_id = '{nkey}' AND e.target_id IN ({seed_ids_sql})))
                          AND 'game:' || g_other.id != '{nkey}'
                          AND e.relation_type = 'similar_to'
                        LIMIT 1;
                        """
                    ).fetchone()
                    if direct_game_edge:
                        _, w, src_name = direct_game_edge
                        pct = int(round(w / 2.0 * 100))
                        game_explanation.append(f"Co-jogabilidade com {src_name} ({pct}%)" if is_pt else f"Co-played with {src_name} ({pct}%)")

                    # 1. Desenvolvedor ou estúdio compartilhado com a semente
                    shared_dev = conn.execute(
                        f"""
                        SELECT d.name
                        FROM graph_semantic_edges e1
                        JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                        JOIN game_developers d ON 'developer:' || d.id = e1.target_id
                        WHERE e1.source_id IN ({seed_ids_sql})
                          AND e2.source_id = '{nkey}'
                          AND e1.relation_type = 'developed_by'
                        LIMIT 1;
                        """
                    ).fetchone()
                    if shared_dev:
                        game_explanation.append(f"Mesmo estúdio ({shared_dev[0]})" if is_pt else f"Same studio ({shared_dev[0]})")

                    # 2. Tags estéticas e mecânicas compartilhadas (excluindo ruídos genéricos)
                    shared_game_tags = conn.execute(
                        f"""
                        SELECT k.name
                        FROM graph_semantic_edges e1
                        JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                        JOIN keywords k ON 'keyword:' || k.id = e1.target_id
                        WHERE e1.source_id IN ({seed_ids_sql})
                          AND e2.source_id = '{nkey}'
                          AND e1.relation_type = 'has_tag'
                          AND e2.relation_type = 'has_tag'
                          AND k.name NOT IN ('single-player', 'multiplayer', 'steam cloud', 'steam achievements', 'family sharing', 'steam trading cards')
                        LIMIT 3;
                        """
                    ).fetchall()
                    if shared_game_tags:
                        tag_str = ", ".join(f"#{t[0]}" for t in shared_game_tags)
                        game_explanation.append(f"Tags em comum: {tag_str}" if is_pt else f"Shared tags: {tag_str}")
                    else:
                        # Fallback se só tiver tags amplas
                        any_tags = conn.execute(
                            f"""
                            SELECT k.name
                            FROM graph_semantic_edges e1
                            JOIN graph_semantic_edges e2 ON e1.target_id = e2.target_id
                            JOIN keywords k ON 'keyword:' || k.id = e1.target_id
                            WHERE e1.source_id IN ({seed_ids_sql})
                              AND e2.source_id = '{nkey}'
                              AND e1.relation_type = 'has_tag'
                            LIMIT 2;
                            """
                        ).fetchall()
                        if any_tags:
                            game_explanation.append(f"Tags: {', '.join(f'#{t[0]}' for t in any_tags)}")

                    default_game_reason = "Afinidade de jogabilidade e universo temático" if is_pt else "Gameplay and thematic universe affinity"
                    full_game_reason = " • ".join(game_explanation) if game_explanation else default_game_reason
                    game_subtitle = f"Jogo ({row[1] or 'N/A'})" if is_pt else f"Game ({row[1] or 'N/A'})"

                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "game",
                        "subtitle": game_subtitle,
                        "score": round(score * 1000, 2),
                        "reason": full_game_reason,
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "game",
                        "val": 12.0,
                    })
                    recs_count += 1


            elif prefix in ("keyword", "person", "developer", "author") and bridge_count < 12:
                # Nós ponte explicativos no grafo
                label = ""
                if prefix == "keyword":
                    krow = conn.execute("SELECT name FROM keywords WHERE id = ?", [nid]).fetchone()
                    if krow:
                        label = f"#{krow[0]}"
                elif prefix == "person":
                    prow = conn.execute("SELECT name FROM people WHERE id = ?", [nid]).fetchone()
                    if prow:
                        label = prow[0]
                elif prefix == "developer":
                    drow = conn.execute("SELECT name FROM game_developers WHERE id = ?", [nid]).fetchone()
                    if drow:
                        label = drow[0]
                elif prefix == "author":
                    arow = conn.execute("SELECT name FROM book_authors WHERE id = ?", [nid]).fetchone()
                    if arow:
                        label = arow[0]

                if label and not any(n["id"] == nkey for n in graph_nodes):
                    graph_nodes.append({
                        "id": nkey,
                        "label": label,
                        "type": prefix,
                        "val": 8.0,
                    })
                    bridge_count += 1

        # 4. Extrai conexões entre os nós exibidos para o force-graph
        all_graph_node_ids = {n["id"] for n in graph_nodes}
        if all_graph_node_ids:
            nodes_sql = ", ".join(f"'{nid}'" for nid in all_graph_node_ids)
            edge_query = f"""
            SELECT source_id, target_id, relation_type, base_weight
            FROM graph_semantic_edges
            WHERE source_id IN ({nodes_sql}) AND target_id IN ({nodes_sql})
            ORDER BY 
                CASE WHEN relation_type LIKE '%soundtrack%' THEN 1
                     WHEN relation_type = 'similar_to' THEN 2
                     WHEN relation_type = 'performed_by' THEN 3
                     WHEN relation_type = 'has_director' THEN 4
                     WHEN relation_type = 'developed_by' THEN 5
                     ELSE 6 END,
                base_weight DESC;
            """
            for src, dst, rel, w in conn.execute(edge_query).fetchall():
                graph_links.append({
                    "source": src,
                    "target": dst,
                    "label": rel,
                    "weight": w,
                })

            # Higienização topológica: elimina nós ponte que ficaram com grau 0 (sem arestas no subgrafo exibido)
            connected_node_ids = set()
            for l in graph_links:
                connected_node_ids.add(l["source"])
                connected_node_ids.add(l["target"])

            # Mantém seeds e recomendações sempre, mas se for nó ponte intermediário (keyword/person/author) sem conexão, remove
            graph_nodes = [
                n for n in graph_nodes
                if n["id"] in connected_node_ids or n["type"] == "seed" or any(r["id"] == n["id"] for r in results)
            ]

        # 5. Enriquece cada nó do grafo com suas tags e metadados descritivos
        is_pt = (req.lang == "pt")
        for n in graph_nodes:
            n_id = n["id"]
            if ":" not in n_id:
                continue
            pfx, raw_id = n_id.split(":", 1)
            try:
                nid = int(raw_id)
            except ValueError:
                continue

            node_tags: List[str] = []
            node_desc = ""

            if pfx == "artist":
                trows = conn.execute(
                    "SELECT k.name FROM artist_tags t JOIN keywords k ON k.id = t.keyword_id WHERE t.artist_id = ? ORDER BY t.weight DESC LIMIT 12",
                    [nid],
                ).fetchall()
                node_tags = [f"#{r[0]}" for r in trows]
                node_desc = "Artista musical" if is_pt else "Musical Artist"

            elif pfx == "track":
                # Tags diretas da faixa ou do artista
                trows = conn.execute(
                    "SELECT k.name FROM graph_semantic_edges e JOIN keywords k ON 'keyword:' || k.id = e.target_id WHERE e.source_id = ? AND e.relation_type = 'has_tag' ORDER BY e.base_weight DESC LIMIT 12",
                    [n_id],
                ).fetchall()
                if not trows:
                    trows = conn.execute(
                        "SELECT k.name FROM artist_tags t JOIN keywords k ON k.id = t.keyword_id WHERE t.artist_id = (SELECT artist_id FROM music_tracks WHERE id = ?) ORDER BY t.weight DESC LIMIT 12",
                        [nid],
                    ).fetchall()
                node_tags = [f"#{r[0]}" for r in trows]
                t_info = conn.execute("SELECT a.name, t.tempo, t.energy FROM music_tracks t JOIN music_artists a ON a.id = t.artist_id WHERE t.id = ?", [nid]).fetchone()
                if t_info:
                    bpm_str = f"{round(t_info[1] or 0)} BPM"
                    node_desc = f"Faixa de {t_info[0]} • {bpm_str}" if is_pt else f"Track by {t_info[0]} • {bpm_str}"

            elif pfx == "game":
                trows = conn.execute(
                    "SELECT k.name FROM game_tags t JOIN keywords k ON k.id = t.keyword_id WHERE t.game_id = ? LIMIT 12",
                    [nid],
                ).fetchall()
                node_tags = [f"#{r[0]}" for r in trows]
                grow = conn.execute("SELECT release_year FROM games WHERE id = ?", [nid]).fetchone()
                if grow and grow[0]:
                    node_desc = f"Jogo ({grow[0]})" if is_pt else f"Game ({grow[0]})"

            elif pfx == "movie":
                trows = conn.execute(
                    "SELECT k.name FROM movie_keywords mk JOIN keywords k ON k.id = mk.keyword_id WHERE mk.movie_id = ? LIMIT 12",
                    [nid],
                ).fetchall()
                node_tags = [f"#{r[0]}" for r in trows]
                mrow = conn.execute("SELECT release_year, vote_average FROM movies WHERE id = ?", [nid]).fetchone()
                if mrow:
                    rating_fmt = f"{mrow[1]:.1f}" if mrow[1] is not None else "N/A"
                    node_desc = f"Filme ({mrow[0] or 'N/A'}) • Nota {rating_fmt}" if is_pt else f"Movie ({mrow[0] or 'N/A'}) • Rating: {rating_fmt}"

            elif pfx == "book":
                trows = conn.execute(
                    "SELECT k.name FROM book_subjects bs JOIN keywords k ON k.id = bs.keyword_id WHERE bs.book_id = ? LIMIT 12",
                    [nid],
                ).fetchall()
                node_tags = [f"#{r[0]}" for r in trows]
                node_desc = "Livro" if is_pt else "Book"

            elif pfx == "developer":
                node_desc = "Estúdio de Desenvolvimento" if is_pt else "Game Developer Studio"

            elif pfx == "keyword":
                node_desc = "Tag Conceitual" if is_pt else "Conceptual Tag"

            elif pfx == "person":
                node_desc = "Diretor / Compositor / Roteirista" if is_pt else "Director / Composer / Writer"

            n["tags"] = node_tags
            n["desc"] = node_desc


    resp_obj = RecommendationResponse(
        seeds=seeds_resolved,
        results=results,
        graph={"nodes": graph_nodes, "links": graph_links},
    )

    if len(_RECOMMENDATION_CACHE) >= _MAX_CACHE_SIZE:
        # Descarta a chave mais antiga
        oldest_k = next(iter(_RECOMMENDATION_CACHE))
        del _RECOMMENDATION_CACHE[oldest_k]
    _RECOMMENDATION_CACHE[cache_key] = resp_obj

    return resp_obj




# Service Worker and Web App Manifest routes
@app.get("/sw.js")
def serve_sw():
    sw_file = STATIC_DIR / "sw.js"
    if sw_file.exists():
        return FileResponse(
            sw_file,
            media_type="application/javascript",
            headers={
                "Service-Worker-Allowed": "/",
                "Cache-Control": "no-cache, no-store, must-revalidate",
            },
        )
    return {"error": "sw.js not found"}


@app.get("/manifest.json")
def serve_manifest():
    manifest_file = STATIC_DIR / "manifest.json"
    if manifest_file.exists():
        return FileResponse(
            manifest_file,
            media_type="application/manifest+json",
            headers={"Cache-Control": "public, max-age=3600"},
        )
    return {"error": "manifest.json not found"}


@app.get("/favicon.ico")
def serve_favicon():
    icon_file = STATIC_DIR / "icons" / "icon-192.png"
    if icon_file.exists():
        return FileResponse(icon_file, media_type="image/png")
    return FileResponse(STATIC_DIR / "icons" / "icon.svg", media_type="image/svg+xml")


# Serve a SPA estática
if STATIC_DIR.exists():
    app.mount("/static", StaticFiles(directory=str(STATIC_DIR)), name="static")


@app.get("/")
def serve_index():
    index_file = STATIC_DIR / "index.html"
    if index_file.exists():
        return FileResponse(index_file)
    return {"message": "youmightlike API online. UI static files not yet created."}

