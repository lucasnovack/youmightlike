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
def search_entities(q: str = Query(..., min_length=2), limit: int = 8) -> List[EntityItem]:
    """Busca em tempo real nas 4 mídias (Músicas, Filmes, Jogos e Livros) com fast-path."""
    q_str = q.strip()
    if not q_str:
        return []

    items: List[EntityItem] = []
    seen = set()

    with get_db() as conn:
        # 1. Artistas (Lookup exato + prefixo)
        variants = list(dict.fromkeys([q_str, q_str.title(), q_str.capitalize()]))
        art_placeholders = ", ".join(["?" for _ in variants])
        art_rows = conn.execute(
            f"""
            SELECT id, name, country, listeners 
            FROM music_artists 
            WHERE name IN ({art_placeholders}) OR name LIKE ? || '%'
            ORDER BY listeners DESC 
            LIMIT 3;
            """,
            variants + [q_str],
        ).fetchall()

        for aid, aname, country, listeners in art_rows:
            nid = f"artist:{aid}"
            if nid not in seen:
                seen.add(nid)
                loc = f" • {country}" if country else ""
                items.append(
                    EntityItem(
                        id=nid,
                        name=aname,
                        type="artist",
                        subtitle=f"Artista Musical{loc}",
                    )
                )

        # 2. Filmes
        mov_rows = conn.execute(
            """
            SELECT id, title, release_year, vote_average 
            FROM movies 
            WHERE title = ? OR title LIKE ? || '%'
            ORDER BY vote_count DESC NULLS LAST
            LIMIT 3;
            """,
            [q_str, q_str],
        ).fetchall()

        for mid, mtitle, myear, mvote in mov_rows:
            nid = f"movie:{mid}"
            if nid not in seen:
                seen.add(nid)
                year_str = f" ({myear})" if myear else ""
                score_str = f" • ⭐ {mvote}" if mvote else ""
                items.append(
                    EntityItem(
                        id=nid,
                        name=mtitle,
                        type="movie",
                        subtitle=f"Filme{year_str}{score_str}",
                    )
                )

        # 3. Jogos
        game_rows = conn.execute(
            """
            SELECT id, title, release_year 
            FROM games 
            WHERE title = ? OR title LIKE ? || '%'
            ORDER BY positive_reviews DESC NULLS LAST
            LIMIT 2;
            """,
            [q_str, q_str],
        ).fetchall()

        for gid, gtitle, gyear in game_rows:
            nid = f"game:{gid}"
            if nid not in seen:
                seen.add(nid)
                year_str = f" ({gyear})" if gyear else ""
                items.append(
                    EntityItem(
                        id=nid,
                        name=gtitle,
                        type="game",
                        subtitle=f"Jogo{year_str}",
                    )
                )

        # 4. Músicas mais populares com esse título (se couber no limite)
        if len(items) < limit:
            needed = limit - len(items)
            trk_rows = conn.execute(
                """
                SELECT t.id, t.title, ma.name, t.popularity, t.tempo, t.valence, t.energy
                FROM music_tracks t
                JOIN music_artists ma ON ma.id = t.artist_id
                WHERE t.title = ? OR t.title LIKE ? || '%'
                ORDER BY t.popularity DESC
                LIMIT ?;
                """,
                [q_str, q_str, needed],
            ).fetchall()

            for tid, ttitle, taname, tpop, bpm, val, eng in trk_rows:
                nid = f"track:{tid}"
                if nid not in seen:
                    seen.add(nid)
                    vibe = f"{int(round(bpm))} BPM" if bpm else ""
                    items.append(
                        EntityItem(
                            id=nid,
                            name=ttitle,
                            type="track",
                            subtitle=f"Faixa de {taname} ({vibe})",
                            extra={"popularity": tpop, "artist_name": taname},
                        )
                    )

    return items[:limit]


@app.post("/api/recommend", response_model=RecommendationResponse)
def get_recommendations(req: RecommendationRequest) -> RecommendationResponse:
    """Gera recomendações a partir do perfil e monta o subgrafo interativo."""
    if not req.seeds:
        return RecommendationResponse(seeds=[], results=[], graph={"nodes": [], "links": []})

    seed_set = set(req.seeds)
    seeds_resolved: List[EntityItem] = []

    with get_db() as conn:
        # 1. Carrega os metadados dos nós semente
        for s in req.seeds:
            prefix, s_id = s.split(":")
            nid = int(s_id)
            if prefix == "artist":
                row = conn.execute("SELECT name, country FROM music_artists WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="artist", subtitle=row[1]))
            elif prefix == "movie":
                row = conn.execute("SELECT title, release_year FROM movies WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="movie", subtitle=str(row[1])))
            elif prefix == "game":
                row = conn.execute("SELECT title, release_year FROM games WHERE id = ?", [nid]).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="game", subtitle=str(row[1])))
            elif prefix == "track":
                row = conn.execute(
                    "SELECT t.title, ma.name FROM music_tracks t JOIN music_artists ma ON ma.id = t.artist_id WHERE t.id = ?",
                    [nid],
                ).fetchone()
                if row:
                    seeds_resolved.append(EntityItem(id=s, name=row[0], type="track", subtitle=row[1]))

        # 2. Executa a difusão no motor Rust
        # Usamos as arestas de grafo sem as 40M de faixas brutas para ter retorno em ~0.1s
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
            ",".join(req.seeds),
            "--limit",
            str(req.limit * 3),
            "--db-path",
            DB_PATH,
            "--query-edges",
            "SELECT source_id, target_id, base_weight FROM v_graph_edges WHERE relation_type NOT IN ('has_track', 'contains_track', 'released')",
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

        recs_count = 0
        for item in diffusion_results:
            nkey = item["node_key"]
            score = item["score"]
            if nkey in seed_set:
                continue

            prefix, raw_id = nkey.split(":")
            nid = int(raw_id)

            if prefix == "artist":
                row = conn.execute("SELECT name, country FROM music_artists WHERE id = ?", [nid]).fetchone()
                if row:
                    # Busca 1 faixa mais famosa desse artista para tocar/mostrar
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
                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "artist",
                        "subtitle": f"Artista Musical ({row[1] or 'Global'})",
                        "score": round(score * 1000, 2),
                        "top_track": trk_info,
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "artist",
                        "val": 12.0,
                    })
                    recs_count += 1

            elif prefix == "movie":
                row = conn.execute("SELECT title, release_year, vote_average FROM movies WHERE id = ?", [nid]).fetchone()
                if row:
                    results.append({
                        "id": nkey,
                        "title": row[0],
                        "type": "movie",
                        "subtitle": f"Filme ({row[1]}) • ⭐ {row[2]}",
                        "score": round(score * 1000, 2),
                    })
                    graph_nodes.append({
                        "id": nkey,
                        "label": row[0],
                        "type": "movie",
                        "val": 12.0,
                    })
                    recs_count += 1

            elif prefix == "game":
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

            elif prefix in ("keyword", "person") and len(graph_nodes) < 30:
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

                if label:
                    graph_nodes.append({
                        "id": nkey,
                        "label": label,
                        "type": prefix,
                        "val": 7.0,
                    })

            if recs_count >= req.limit:
                break

        # 4. Extrai conexões entre os nós exibidos para o force-graph
        all_graph_node_ids = {n["id"] for n in graph_nodes}
        if all_graph_node_ids:
            nodes_sql = ", ".join(f"'{nid}'" for nid in all_graph_node_ids)
            edge_query = f"""
            SELECT source_id, target_id, relation_type, base_weight
            FROM v_graph_edges
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
