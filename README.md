# youmightlike

Motor de recomendação cultural cruzada entre **Filmes, Músicas, Livros e Jogos**. 

Em vez de silos isolados (Spotify só recomenda música, Steam só recomenda jogo, Letterboxd só recomenda filme), o `youmightlike` conecta essas mídias em um grafo de afinidade estética e difunde o gosto do usuário usando algoritmos clássicos de grafos.

**Zero LLMs, zero APIs pagas, 100% local e determinístico.**

---

## Por que não usar LLM?

1. **Custo e Dependência:** Fazer chamadas de API proprietária para cada consulta de recomendação é caro, lento e quebra offline.
2. **Explicabilidade:** LLMs alucinam conexões e sofrem de esquecimento/bias por popularidade na web. No grafo, cada sugestão tem uma rota clara e auditável (*"Recomendado porque você marcou o Filme X e o Artista Y, conectados pelo Compositor Z e pela estética Cyberpunk"*).
3. **Desempenho:** Personalized PageRank e álgebra linear esparsa em CPU rodam em milissegundos sobre centenas de milhares de nós sem queimar GPU ou gastar um centavo.

---

## Como Funciona

```
[ Datasets Abertos (IMDb, Steam, Goodreads, MusicBrainz) ]
                           │
                 Ingestão Nativa (Parquet/TSV)
                           ▼
          ┌──────────────────────────────────┐
          │   DuckDB Feature Store Local     │
          │  492k nós │ 1.72M arestas ativas │
          └─────────────────┬────────────────┘
                            │
               Zero-Copy / Views Ponderadas (IDF)
                            ▼
          ┌──────────────────────────────────┐
          │   Motor de Difusão Estatística   │
          │  (Personalized PageRank na CPU)  │
          └─────────────────┬────────────────┘
                            │
                            ▼
           Recomendações com Caminho Explicado
```

### Ponderação de Arestas (Adamic-Adar / IDF)
Tags e categorias genéricas (como *"Ação"*, *"Rock"*, *"Ficção"*) saturam grafos e atuam como sumidouros de probabilidade, gerando apenas sugestões óbvias. 

Para resolver isso de forma determinística, as arestas passam por uma view no DuckDB (`v_weighted_graph_edges`) que atenua o peso da conexão pelo inverso do logaritmo do grau do nó de destino:

$$\text{peso}(u, v) = \frac{\text{base\_weight}(u, v)}{\ln(1 + \text{grau}(v))}$$

Conexões específicas (uma tag de nicho, um compositor de trilha ou um autor cult) transferem muito mais energia no grafo do que categorias amplas.

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
| **Artistas Musicais** (`music_artists`) | **126.791** | MusicBrainz (CC0) + Spotify |
| **Álbuns Musicais** (`music_albums`) | **63.289** | Discografias |
| **Faixas / Músicas** (`music_tracks`) | **85.889** | Spotify Tracks com Audio Features (BPM, Valência, Energia) |
| **Tags de Música** (`artist_tags`) | **475.786** | Microgêneros e folksonomia |
| **Similaridades Musicais** (`artist_similarities`) | **61.680** | Last.fm Co-listening Graph (HetRec) |
| **Keywords Globais** (`keywords`) | **47.890** | Vocabulário temático unificado |
| **Total de Nós (`v_graph_nodes`)** | **492.804** | Consulta analítica em ~0.01s |
| **Total de Arestas (`v_graph_edges`)** | **1.726.926** | Ponderação e consulta em ~1.0s |

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

### 4. Consultar o Grafo
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
