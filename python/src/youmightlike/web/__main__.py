"""Entrypoint CLI para iniciar o servidor web nativo em Rust (Axum) do youmightlike."""

from __future__ import annotations

import os
import sys
from pathlib import Path

RUST_SERVER_BIN = Path("target/release/youmightlike-server")


def main() -> None:
    if not RUST_SERVER_BIN.exists():
        print(
            f"[youmightlike] Erro: binário '{RUST_SERVER_BIN}' não encontrado. "
            "Compile com: cargo build --release",
            file=sys.stderr,
        )
        sys.exit(1)

    args = [str(RUST_SERVER_BIN)] + sys.argv[1:]
    os.execv(str(RUST_SERVER_BIN), args)


if __name__ == "__main__":
    main()
