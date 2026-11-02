# youmightlike

Sistema de recomendação transmidiática que conecta **Cinema, Música, Literatura e Games** em um grafo unificado de afinidade estética.

Em vez de silos isolados por mídia, o `youmightlike` projeta relações culturais (direção, trilhas sonoras, literatura de base, estúdios, microgêneros e tags conceituais) sobre um grafo semântico no DuckDB, calculando recomendações por difusão estatística (Personalized PageRank) via motor nativo em Rust.

---

## Princípios Técnicos

* **Explicabilidade:** Cada recomendação mantém uma rota rastreável no grafo (*ex.: Filme A conectado ao Artista B pelo Compositor C e pela estética Cyberpunk*).
* **Eficiência Computacional:** Álgebra linear esparsa em CPU via matriz CSR paralelizada com Rayon, executando em sub-segundo sem dependência de serviços externos.
* **Privacidade e Autonomia:** Armazenamento colunar local no DuckDB com zero chamadas remotas de inferência ou telemetria em runtime.

---

## Arquitetura

```
[ Datasets Abertos (IMDb, Steam, Goodreads, MusicBrainz, Spotify) ]
                               │
                      Ingestão Colunar (Parquet/TSV)
                               ▼
              ┌──────────────────────────────────┐
              │   DuckDB Feature Store Local     │
              │  3.58M arestas semânticas ativas │
              └─────────────────┬────────────────┘
                                │
                    Zero-Copy / Views Ponderadas
                                ▼
              ┌──────────────────────────────────┐
              │   Motor de Difusão em Rust       │
              │  (Bidirectional PPR / CSR Graph) │
              └─────────────────┬────────────────┘
                                │
                                ▼
               Subgrafo Explicativo & Recomendações
```

### Ponderação de Arestas (Adamic-Adar / IDF)

Para evitar que categorias amplas (*Ação*, *Rock*, *Ficção*) atuem como sumidouros de probabilidade e saturem o cálculo com sugestões óbvias, a view `v_weighted_graph_edges` atenua o peso de cada aresta com base no inverso do logaritmo do grau do nó de destino:

$$\text{peso}(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{grau}(v))}$$

Conexões específicas e raras (um subgênero de nicho, compositor ou estúdio autoral) retêm alta transferência de energia, enquanto nós de vocabulário massivo sofrem penalização proporcional.


---

## Volume Atual do Grafo (DuckDB)

Todo o grafo roda localmente em um arquivo único (`data/processed/taste_graph.duckdb`):

| Domínio / Tabela | Registros | Fonte / Licença |
| :--- | :---: | :--- |
| **Filmes & Animes** (`movies`) | **34.188** | IMDb Datasets + MyAnimeList Top Anime |
| **Pessoas & Estúdios** (`people`) | **37.498** | Diretores, roteiristas e estúdios de animação |
| **Créditos & Produção** (`movie_credits`) | **106.423** | Direção, roteiro e estúdios (`produced_by`) |
| **Keywords de Filmes/Animes** (`movie_keywords`) | **239.528** | MovieLens Tag Genome + MyAnimeList |
| **Jogos** (`games`) | **20.064** | Steam Dataset + Clássicos de Console (Nintendo/PS/Sega) |
| **Desenvolvedores** (`game_developers`) | **9.250** | Estúdios e publicadoras (PC e Consoles) |
| **Tags de Jogos** (`game_tags`) | **133.983** | Tags comunitárias e plataformas clássicas |
| **Livros** (`books`) | **38.203** | Goodreads 100k (CC0 Parquet) |
| **Autores de Livros** (`book_authors`) | **29.742** | Autores e co-autores |
| **Temas Literários** (`book_subjects`) | **400.285** | Tópicos e categorias |
| **Artistas Musicais** (`music_artists`) | **3.257.918** | MusicBrainz + Spotify Massivo (56M) |
| **Álbuns Musicais** (`music_albums`) | **18.034.542** | Discografias completas e singles mundiais |
| **Faixas / Músicas** (`music_tracks`) | **40.418.286** | Spotify Tracks com Audio Features (BPM, Valência, Energia) |
| **Tags de Música** (`artist_tags`) | **485.064** | Microgêneros e folksonomia |
| **Similaridades Musicais** (`artist_similarities`) | **61.680** | Last.fm Co-listening Graph (HetRec) |
| **Keywords Globais** (`keywords`) | **48.263** | Vocabulário temático unificado |
| **Total de Nós (`v_graph_nodes`)** | **61.927.954** | Consulta analítica no DuckDB |
| **Total de Arestas (`v_graph_edges`)** | **99.278.228** | Quase 100M de arestas ativas ponderadas |

---

## Estrutura do Repositório

```
youmightlike/
├── crates/
│   └── engine/             # Core de difusão em Rust (PPR, Petgraph, Rayon)
├── python/
│   ├── src/youmightlike/
│   │   ├── connectors/     # Clientes pontuais (Wikidata, OpenLibrary, MusicBrainz)
│   │   └── storage/        # Schema SQL, conexão DuckDB e scripts de ingestão bulk
├── data/                   # Ignorado no git
│   ├── raw/                # Dumps compactados (IMDb, Steam, Goodreads, MusicBrainz, Last.fm, Games)
│   └── processed/          # taste_graph.duckdb
└── docs/                   # Documentação detalhada da arquitetura e datasets
```

---

## Como Rodar Localmente

### 1. Pré-requisitos
* Python 3.11+
* Rust & Cargo (para o engine compilado)

### 2. Configurar o Ambiente Python
```bash
# Criar e ativar o ambiente virtual
python3 -m venv .venv
source .venv/bin/activate

# Instalar o pacote em modo editável
pip install -e python/
```

### 3. Rodar as Ingestões em Massa (Opcional se já tiver o banco gerado)
O banco DuckDB é gerado em segundos lendo diretamente arquivos colunares:
```bash
# Ingestão de filmes (IMDb TSVs)
python python/src/youmightlike/storage/ingest_imdb.py

# Ingestão de semântica e tags de filmes (MovieLens Tag Genome)
python python/src/youmightlike/storage/ingest_movielens.py

# Ingestão de animes, estúdios e temas (MyAnimeList)
python python/src/youmightlike/storage/ingest_anime.py

# Ingestão de jogos modernos (Steam Parquet)
python python/src/youmightlike/storage/ingest_steam.py

# Ingestão de clássicos de consoles (Nintendo, PlayStation, Sega)
python python/src/youmightlike/storage/ingest_console_games.py

# Ingestão de livros (Goodreads Parquet)
python python/src/youmightlike/storage/ingest_books.py

# Ingestão de discografia e artistas (MusicBrainz Parquet + Spotify CSV)
python python/src/youmightlike/storage/ingest_music.py

# Ingestão de faixas musicais individuais com audio features (Spotify)
python python/src/youmightlike/storage/ingest_tracks.py

# Ingestão de similaridades diretas entre artistas (Last.fm Graph)
python python/src/youmightlike/storage/ingest_lastfm_similars.py
```

### 4. Iniciar a Interface Web Interativa (Force-Graph UI)
Para rodar a aplicação web com o visualizador de grafos topológicos, busca instantânea e recomendações bilingues:
```bash
# Iniciar o servidor FastAPI com live-reload na porta 8000
python -m youmightlike.web

# Ou diretamente via uvicorn
uvicorn youmightlike.web.server:app --reload --port 8000
```
Abra o navegador em `http://localhost:8000`:
- **Barra de perfil inferior:** Adicione sementes favoritas (músicas, jogos, filmes ou artistas).
- **Campo Topológico:** Explore as arestas semânticas interativas renderizadas com Force-Graph.
- **Seletor [ PT | EN ]:** Alterne entre português e inglês instantaneamente.
- **Shift + Click:** Inspecione e realce caminhos de conexão direta e pontes mútuas entre múltiplos nós.

### 5. Consultar o Grafo via Python / CLI
Você pode inspecionar o grafo direto no terminal ou conectando o DBeaver em modo `read_only`:
```python
import duckdb

conn = duckdb.connect("data/processed/taste_graph.duckdb", read_only=True)
print("Nós:", conn.execute("SELECT count(*) FROM v_graph_nodes").fetchone()[0])
print("Arestas:", conn.execute("SELECT count(*) FROM v_graph_edges").fetchone()[0])
```

---

## Licença

MIT. Os datasets externos respeitam seus respectivos termos abertos (CC0, ODC-By, e licenças de dados comunitários).
