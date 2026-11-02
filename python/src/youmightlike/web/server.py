"""Servidor FastAPI e API REST para a UI do youmightlike."""

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
    description="Motor de Descoberta Cultural Transmidiático e Grafo de Gosto Pessoal",
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


@app.post("/api/recommend", response_model=RecommendationResponse)
def get_recommendations(req: RecommendationRequest) -> RecommendationResponse:
    """Gera recomendações a partir do perfil e monta o subgrafo interativo."""
    if not req.seeds:
        return RecommendationResponse(seeds=[], results=[], graph={"nodes": [], "links": []})

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
                        "type": "tag",
                        "val": 9.0,
                    })

        recs_count = 0
        bridge_count = 0
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
                    if direct_edge:
                        rel, w, src_name = direct_edge
                        if rel == "similar_to":
                            pct = int(round(w * 100))
                            explanation_parts.append(f"Co-escuta com {src_name} ({pct}%)")
                        else:
                            explanation_parts.append(f"Conexão direta com {src_name}")

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

                    full_reason = " • ".join(explanation_parts) if explanation_parts else "Difusão estocástica topológica"

                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "artist",
                        "subtitle": f"Artista Musical ({row[1] or 'Global'})",
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
                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "movie",
                        "subtitle": f"Movie ({row[1]}) • Rating: {row[2]}",
                        "score": round(score * 1000, 2),
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
                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "game",
                        "subtitle": f"Jogo ({row[1]})",
                        "score": round(score * 1000, 2),
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "game",
                        "val": 12.0,
                    })
                    recs_count += 1

            elif prefix in ("keyword", "person") and bridge_count < 8:
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

                if label and not any(n["id"] == nkey for n in graph_nodes):
                    graph_nodes.append({
                        "id": nkey,
                        "label": label,
                        "type": prefix,
                        "val": 7.0,
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
            LIMIT 80;
            """
            for src, dst, rel, w in conn.execute(edge_query).fetchall():
                graph_links.append({
                    "source": src,
                    "target": dst,
                    "label": rel,
                    "weight": w,
                })

    return RecommendationResponse(
        seeds=seeds_resolved,
        results=results,
        graph={"nodes": graph_nodes, "links": graph_links},
    )


# Serve a SPA estática
if STATIC_DIR.exists():
    app.mount("/static", StaticFiles(directory=str(STATIC_DIR)), name="static")


@app.get("/")
def serve_index():
    index_file = STATIC_DIR / "index.html"
    if index_file.exists():
        return FileResponse(index_file)
    return {"message": "youmightlike API online. UI static files not yet created."}
