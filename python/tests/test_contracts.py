"""Unit and integration contract tests for youmightlike Python layer."""

from youmightlike.web.server import (
    EntityItem,
    GraphEdge,
    GraphNode,
    RecommendationRequest,
    RecommendationResponse,
)


def test_recommendation_request_defaults():
    req = RecommendationRequest(seeds=["movie:1"])
    assert req.seeds == ["movie:1"]
    assert req.negative_seeds == []
    assert req.known_items == []
    assert req.target_type is None
    assert req.limit == 12
    assert req.lang == "pt"
    assert req.beta == 1.2


def test_recommendation_request_custom():
    req = RecommendationRequest(
        seeds=["game:1", "artist:2"],
        negative_seeds=["artist:3"],
        known_items=["game:2"],
        target_type="game",
        limit=24,
        lang="en",
        beta=1.5,
    )
    assert req.limit == 24
    assert req.lang == "en"
    assert req.target_type == "game"
    assert req.beta == 1.5


def test_entity_item_serialization():
    item = EntityItem(
        id="movie:42",
        name="Blade Runner",
        type="movie",
        subtitle="1982 • Dir. Ridley Scott",
        extra={"year": 1982, "rating": 8.1},
    )
    data = item.model_dump()
    assert data["id"] == "movie:42"
    assert data["name"] == "Blade Runner"
    assert data["type"] == "movie"
    assert data["extra"]["year"] == 1982


def test_graph_node_and_edge_serialization():
    node = GraphNode(id="game:1", label="Skyrim", type="game", val=14.0)
    assert node.id == "game:1"
    assert node.val == 14.0

    edge = GraphEdge(source="game:1", target="developer:1", label="developed_by", weight=2.0)
    assert edge.source == "game:1"
    assert edge.target == "developer:1"
    assert edge.weight == 2.0


def test_recommendation_response_structure():
    resp = RecommendationResponse(
        seeds=[EntityItem(id="movie:1", name="Matrix", type="movie")],
        negative_seeds=[],
        known_items_count=0,
        results=[],
        graph={"nodes": [], "links": []},
    )
    assert len(resp.seeds) == 1
    assert resp.seeds[0].id == "movie:1"
    assert resp.graph["nodes"] == []
    assert resp.graph["links"] == []
