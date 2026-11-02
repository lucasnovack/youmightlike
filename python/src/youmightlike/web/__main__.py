"""Entrypoint CLI para iniciar o servidor web do youmightlike."""

import uvicorn


def main():
    print("[youmightlike] Starting server on http://127.0.0.1:8000 ...")
    uvicorn.run("youmightlike.web.server:app", host="127.0.0.1", port=8000, reload=True)


if __name__ == "__main__":
    main()
