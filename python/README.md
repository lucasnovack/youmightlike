# youmightlike - Python Data Layer

Data ingestion pipelines, public dataset connectors, and local DuckDB feature store loaders for `youmightlike`.

## Overview

This module provides the data extraction and ingestion tools that build the unified transmedia cultural taste graph in DuckDB:

- **Connectors (`connectors/`):** Fetchers for Wikidata SPARQL (directors, composers, adaptations), MusicBrainz (artists, microgenres), and OpenLibrary (books, authors).
- **Storage & Ingestion (`storage/`):** High-speed batch loaders converting columnar raw files (Parquet and gzip TSV) into canonical tables and SQL graph views (`v_graph_nodes`, `v_graph_edges`, `v_weighted_graph_edges`).
- **REST Service & Static Assets (`web/`):** FastAPI app and static frontend assets for the interactive HTML5 Canvas constellation UI.

## Installation

```bash
# Editable install within virtual environment
pip install -e "python/[dev]"
```

## Running Tests & Linters

```bash
ruff check python/
pytest python/tests
```
