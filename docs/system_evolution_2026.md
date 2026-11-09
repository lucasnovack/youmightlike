# System Architecture Updates & Technical Evolution (November 2026)

This document records the architectural evolution, critical transmedia topology fixes, the native bidirectional Rust diffusion engine, and the interactive graph visualization engine implemented in `youmightlike`.

---

## 1. Overview of Key Advancements

The system evolved from an initial prototype into a high-fidelity cultural discovery and recommendation platform:
* **Semantic Graph Scale:** 1,956,347 active semantic edges in DuckDB (`graph_semantic_edges`), expanding to 3,912,424 in-memory stochastic transitions across 357,643 connected nodes.
* **Native Rust Bidirectional Engine:** Parallelized Personalized PageRank (PPR) using Rayon, operating over Compressed Sparse Row (CSR) matrices with symmetric diffusion mirroring.
* **Studio Minimalist Interface:** Modern Single-Page Application (SPA) with FastAPI/Axum, JetBrains Mono typography, calibrated sober color palette, and zero visual noise or emojis.
* **Interactive Topological Visualization:** Native 2D Canvas ForceGraph renderer with 1-hop neighborhood highlighting, multi-node selection (Shift+Click), and structural bridge inspection.

---

## 2. Bidirectional Diffusion Engine in Rust (`crates/engine`)

### 2.1 The Edge Orientation Challenge
In the original directed graph, edges such as `movie -> keyword` or `artist -> tag` flowed strictly from works to concepts. In random walk diffusion (PageRank):
* Energy radiating from an active seed (*e.g. Donnie Darko*) flowed into concept tags (`#cult`, `#soundtrack`).
* Without reverse edges (`tag -> movie`), probability mass pooled at conceptual leaves and failed to flow back into related films, albums, or games sharing the same tag.

### 2.2 Solution: Symmetric Mirroring in CSR Matrices
During graph loading (`crates/engine/src/graph.rs`):
* Every directed semantic edge `(u, v)` automatically receives its symmetric counterpart `(v, u)` during adjacency list construction.
* Stochastic row normalization `∑_j P_{ij} = 1` is computed after bidirectional edge inclusion, preserving mathematical convergence.
* PPR diffusion now transfers probability mass fluidly across different media domains via shared conceptual nodes.

---

## 3. Transmedia Topology & Structural Anchors

### 3.1 Transmedia Disconnection Resolution (e.g., Skyrim + Booka Shade)
* **Symptom:** When combining cross-domain seeds in a profile (*e.g., the video game The Elder Scrolls V: Skyrim and electronic duo Booka Shade*), the Skyrim cluster appeared visually isolated on the canvas with zero incoming or outgoing edges.
* **Root Cause:** The backend limited intermediate bridge nodes to 8 global slots and only accepted `keyword:` and `person:` prefixes. Electronic scene tags consumed all 8 slots, excluding game developer nodes (`developer:Bethesda Game Studios`) and game tags (`keyword:rpg`). Without Bethesda in the displayed node set, the edge query `WHERE source_id IN (nodes) AND target_id IN (nodes)` returned 0 links for Skyrim.
* **Dedicated Seed Structural Anchors:**
  - The graph builder now reserves **dedicated structural anchors for each active profile seed** (`seeds_resolved`), ensuring foundational connectors (studio for games, director/composer for movies, primary genre for music) are always present in the visual subgraph.
  - Bridge parsing expanded to support `developer:` and `author:` prefixes.
  - Result: Skyrim connects reliably to Bethesda Game Studios and kindred lineage titles (*Morrowind*, *Oblivion*, *Fallout 3*, *Fallout 4*, *Starfield*).

### 3.2 Direct Soundtrack Links (*Donnie Darko* ⇄ *The Killing Moon*)
* Created canonical `movie_soundtrack_links` table in DuckDB.
* *Donnie Darko* (`movie:22697`) is mapped with maximum semantic weight directly to indexed instances of *The Killing Moon* (`track:12883639` and `track:60808`) via `soundtrack_track` edges, and to band *Echo & the Bunnymen* (`artist:927`) via `soundtrack_featured`.
* Soundtrack edges receive prioritized rendering and immediate visual luminescence when either node is selected.

---

## 4. Semantic Tag Densification (Target: ≥ 10 Tags per Entity)

To prevent niche entities from becoming isolated or receiving disproportionate weights, an enrichment policy was established ("*expand coverage without removing entities*"):
1. **Electronic Music & Tech House:** Ingestion of specific tags for electronic producers and DJs (*e.g., Mochakk, Beltran, Vintage Culture, Booka Shade, Solomun*), ensuring descriptors like `#tech-house`, `#deep-house`, `#groove`, `#minimal`, `#underground`.
2. **Classic Bands & Legacy Artists:** Enriched with subgenres, era, and atmosphere (*e.g., Echo & the Bunnymen expanded from sparse tags to 40 descriptors including #post-punk, #goth, #dark-wave, #80s, #melancholic, #coldwave*).
3. **Prioritized Name Resolution:** In search autocomplete and seed resolution, canonical individual entities (*e.g., Massive Attack*) receive positive tie-breaking by degree/relevance to prevent remixes or collaborations from obscuring parent entities.

---

## 5. Graphical Interface & Graph Interactivity

### 5.1 Neighborhood Highlighting (Single & Multi-Node)
The Canvas renderer in `python/src/youmightlike/web/static/js/constellation.js` employs deterministic focus logic:
* **Single Click Selection:**
  - The clicked node receives a dual highlight ring using palette tones (`#e84855` coral and `#182825` onyx) and an expanded radius.
  - Direct 1-hop neighbors retain 100% opacity with visible text labels.
  - Disconnected nodes and edges attenuate to reduced opacity (`globalAlpha = 0.12`).
* **Multi-Node Selection (Shift + Click):**
  - Allows simultaneous inspection of multiple nodes (*e.g., comparing Donnie Darko and The Killing Moon, or Skyrim and Fallout 3*).
  - The union of neighbors from all selected nodes is preserved.
  - Edges directly linking selected nodes illuminate in coral with tripled stroke width and accelerated particle animation.
* **Inspector Card:**
  - Floating panel in the upper-right corner listing inspected items, mutual connection count, and reset shortcut (`Esc` or `[clear]` button).

### 5.2 Media Type Filter in Search
The search input includes a media filter selector (`all`, `movie`, `game`, `book`, `artist`, `track`), ensuring precision when searching for ambiguous terms (*e.g., finding the game Skyrim rather than tracks from its soundtrack*).

### 5.3 Color Palette & Typography
The interface uses a 5-tone palette calibrated for high contrast and technical sobriety:
* **Deep Onyx (`#182825`):** Primary text, structural rings, and borders.
* **Warm Eggshell (`#faf3dd`):** Card backgrounds, badges, and secondary surfaces.
* **Sage Green (`#4e937a`):** Conceptual connectors, tags, and moderate weights.
* **Accent Coral (`#e84855`):** Focus highlights, seed nodes, and high-flux edges.
* **Steel Blue (`#2660a4`):** Canonical identifiers and artistic domain nodes.
* **Typography:** `JetBrains Mono` across the entire visual hierarchy.
* **Standardization:** Complete elimination of emojis in console logs, CLI outputs, and visual elements, ensuring a refined Unix/Apple aesthetic.
 
---

## 6. Algorithmic Calibration, Noise Attenuation & High-DPI Ticket (November 2026)

### 6.1 Meta-Tag Wormhole Attenuation & Acoustic Leak Mitigation
* **Symptom:** Unrelated cross-media recommendations (*e.g., grunge film Singles (1992) appearing for high fantasy seeds like The Elder Scrolls V: Skyrim and Jeremy Soule*) due to generic structural descriptors (`#soundtrack`, `#score`, `#vgm`, `#music`, `#game composer`).
* **Root Cause:** Soundtrack link weights in `graph_semantic_edges` reached 4.0, while genre/thematic tags were ~0.12. Furthermore, `#soundtrack` was not filtered from shared tag explanations, creating an artificial cross-domain probability wormhole.
* **Resolution:**
  - Broad structural metadata tags (72 acoustic/soundtrack keywords and 45 country/language tags) are attenuated by 98% (`base_w *= 0.02`) during CSR matrix construction in `crates/engine/src/graph.rs`.
  - `is_steam_noise_tag` in `crates/engine/src/recommendation/context.rs` filters any tag containing `soundtrack`, `score`, `ost`, `vgm`, `composer`, preventing them from acting as thematic bridges.
  - Quadratic multi-tag confluence boost ($1.0 + N^2 \times 5.0$) implemented in `movies.rs` and `series.rs`, elevating works with multiple shared thematic anchors (e.g. fantasy, medieval, magic, dragons) while dampening zero-thematic-tag candidates by 85% (`score *= 0.15`).

### 6.2 Quota Calibration & Cultural Passport Polish
* **Balanced Recommendation Quotas:** Default exploration pool calibrated to 24–28 curated items (4–6 per domain), preventing information overload while preserving cross-domain variety.
* **3x Retina Cultural Ticket:** Export resolution upgraded to 3x Retina (`dpr = Math.max(window.devicePixelRatio || 1, 3)`), Apple Passbook-style squircle header emblem, centered 224px Code 128 barcode with taller guard bars, and perforated tear notches eliminating bottom whitespace.

### 6.3 The Boundary: Structural Affinity vs. Atmospheric Resonance (The Interstellar Case Study)
Auditing recommendations generated from the seminal prompt `Interstellar (2014)` identified the clear distinction between topological correctness and atmospheric/emotional cohesion:
* **Organic Confluence:** *Westworld* (shared co-writer Jonathan Nolan, existential sci-fi, temporal distortion, memory ethics) represents optimal graph convergence.
* **Authorial Gravitational Bias:** Works such as *The Dark Knight Rises* and *American Prometheus* surface due to Christopher Nolan's authorial weight. Topologically sound, yet emotionally disparate from cosmic existential solitude (*Arrival*, *Contact*, *Solaris* serve as more intuitive tonal counterparts).
* **Lexical Leakage via Broad Monosemic Tags:** Recommending country/roots rock track *Where There's Smoke There's Whiskey* (Steve Rivers) solely via the isolated tag `#time` underscores the peril of pure lexical tag overlap without harmonic or acoustic anchoring.
* **Architectural Takeaway:** Abstract, broad conceptual tags (`#time`, `#life`, `#love`) must undergo hub dampening analogous to structural meta-tags, demanding quadratic multi-tag concurrence before bridging distant creative domains.

