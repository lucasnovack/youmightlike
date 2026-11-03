from __future__ import annotations

import os
from pathlib import Path
from typing import Generator
import duckdb
from dotenv import load_dotenv

load_dotenv()

DEFAULT_DB_PATH = Path("data/processed/taste_graph.duckdb")


def get_db_path() -> Path:
    env_path = os.getenv("DUCKDB_PATH")
    if env_path:
        return Path(env_path)
    return DEFAULT_DB_PATH


class Database:
    """Connection manager and query runner for DuckDB."""

    def __init__(self, db_path: Path | str | None = None, read_only: bool = False):
        if db_path is None:
            self.db_path = get_db_path()
        else:
            self.db_path = Path(db_path)

        self.read_only = read_only
        self._ensure_dir()

    def _ensure_dir(self) -> None:
        if str(self.db_path) != ":memory:":
            self.db_path.parent.mkdir(parents=True, exist_ok=True)

    def connect(self) -> duckdb.DuckDBPyConnection:
        """Returns a new open connection to DuckDB."""
        return duckdb.connect(database=str(self.db_path), read_only=self.read_only)

    def init_schema(self) -> None:
        """Executes the initialization SQL script for canonical schema and graph views."""
        schema_file = Path(__file__).parent / "schema.sql"
        if not schema_file.exists():
            raise FileNotFoundError(f"Schema file not found: {schema_file}")

        sql = schema_file.read_text(encoding="utf-8")
        with self.connect() as conn:
            conn.execute(sql)


def init_db(db_path: Path | str | None = None) -> Database:
    """Utility function to initialize the database and ensure tables/views are created."""
    db = Database(db_path=db_path)
    db.init_schema()
    return db


if __name__ == "__main__":
    db = init_db()
    print(f"[ok] DuckDB initialized at: {db.db_path.resolve()}")
    with db.connect() as conn:
        res = conn.execute(
            "SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = 'main' ORDER BY table_type, table_name"
        ).fetchall()
        tables = [r[0] for r in res if r[1] == 'BASE TABLE']
        views = [r[0] for r in res if r[1] == 'VIEW']
        print(f"Tabelas criadas ({len(tables)}): {tables}")
        print(f"Views criadas ({len(views)}): {views}")
