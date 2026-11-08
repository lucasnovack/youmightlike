#!/usr/bin/env bash
set -euo pipefail

DUCKDB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/lib/duckdb"
mkdir -p "$DUCKDB_DIR"

if [ ! -f "$DUCKDB_DIR/libduckdb.so" ]; then
    echo "[setup] Downloading precompiled DuckDB shared library (~30MB)..."
    curl -sL https://github.com/duckdb/duckdb/releases/download/v2.0.0/duckdb-shared-libs-linux-amd64.tar.gz | tar -xzvf - -C "$DUCKDB_DIR"
    echo "[setup] DuckDB shared library ready in $DUCKDB_DIR"
else
    echo "[setup] DuckDB shared library already present in $DUCKDB_DIR"
fi
