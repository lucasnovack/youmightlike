# Arquitetura do Sistema e Modelo de Grafo

O `youmightlike` opera sobre uma premissa técnica: **bancos relacionais colunares modernos (DuckDB) são feature stores excepcionais para alimentar algoritmos matriciais esparsos em CPU**.

Em vez de manter bancos de grafos dedicados pesados (Neo4j) ou depender de embeddings caros e opacos via APIs de LLM, o sistema estrutura o conhecimento em tabelas relacionais dimensionais e projeta o grafo cultural por meio de views SQL de alto desempenho.

---

## 1. Topologia da Arquitetura

```
┌─────────────────────────────────────────────────────────────┐
│                      FONTES DE DADOS                        │
│   IMDb Dumps   │   Steam Parquet   │  Goodreads  │  Music   │
└──────────────────────────────┬──────────────────────────────┘
                               │
                       DuckDB Ingestion
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                 DUCKDB (Feature Store Local)                │
│  - Tabelas Canônicas: movies, people, games, books, etc.     │
│  - Tabela Central de Vocabulário: keywords                  │
│                                                             │
│  VIEWS DE PROJEÇÃO:                                         │
│    ├─ v_graph_nodes           (Nós unificados: 392k)        │
│    ├─ v_graph_edges           (Arestas tipadas: 1.23M)      │
│    └─ v_weighted_graph_edges  (Arestas com Adamic-Adar/IDF) │
└──────────────────────────────┬──────────────────────────────┘
                               │
                     Zero-Copy / Memory Scan
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                 MOTOR DE DIFUSÃO ESTATÍSTICA                │
│             (Personalized PageRank / Random Walk)           │
│                                                             │
│  - Seeds do usuário: [Filme X, Álbum Y, Game Z]             │
│  - Matriz Esparsa de Adjacência: A[u, v] com pesos IDF      │
│  - Iteração de Potência: p_{k+1} = α · P^T · p_k + (1-α)·s  │
│  - Explicabilidade: Backtracking dos caminhos com maior fluxo│
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Esquema Relacional e Pontes Transmidiáticas

As tabelas canônicas armazenam os registros de cada domínio de mídia. As pontes transmidiáticas são formadas por:

1. **Pessoas e Compositores de Trilha (`movie_credits` ➔ `people` ➔ `music_artists`):**
   Compositores de cinema (ex: Vangelis, Trent Reznor, Hans Zimmer, Ennio Morricone, Joe Hisaishi) conectam filmes diretamente à sua discografia e artistas relacionados (`is_artist` e `has_composer`).
2. **Adaptações Literárias (`movies.based_on_wikidata_id` ➔ `books`):**
   Filmes baseados em livros formam a aresta `based_on`.
3. **Vocabulário Estético Unificado (`keywords`):**
   Gêneros do cinema, tags da comunidade Steam, tópicos literários e microgêneros musicais convergem para a tabela `keywords`. Assim, um filme de ficção científica, um jogo cyberpunk, um livro distópico e um álbum de dark synthwave compartilham os mesmos nós temáticos.

---

## 3. Ponderação de Arestas e Mitigação de Hubs (Adamic-Adar / IDF)

Se todas as conexões tivessem o mesmo peso, tags ultragenéricas (como *"Action"*, *"Rock"*, *"Fiction"*) agiriam como "sumidouros de energia" no grafo, fazendo o PageRank sugerir apenas blockbusters óbvios.

Para resolver isso de forma determinística, a view `v_weighted_graph_edges` aplica atenuação pelo inverso do logaritmo do grau do nó de destino:

$$w_{\text{efetivo}}(u, v) = \frac{w_{\text{base}}(u, v)}{\ln(1 + \text{grau}(v))}$$

* Conexões específicas e raras (ex: tag de nicho *"post-cyberpunk"*, autor cult, compositor específico) mantêm alta transferência de probabilidade.
* Conexões amplas e massivas (ex: *"drama"*, *"music"*) têm seu fluxo atenuado proporcionalmente à sua saturação no grafo.

---

## 4. Projeção das Views de Grafo

O grafo inteiro é mantido como views dinâmicas sobre as tabelas:

* **`v_graph_nodes`:** Projeção unificada com ID composto no formato `<tipo>:<id>` (ex: `movie:183`, `artist:5490`, `book:12401`), nome da entidade, tipo do nó e domínio cultural (`movie`, `game`, `book`, `music`, `concept`).
* **`v_graph_edges`:** União de todas as relações orientadas do sistema (`has_director`, `has_composer`, `is_artist`, `released`, `written_by`, `has_subject`, `has_tag`, `has_keyword`, `developed_by`).
* **`v_weighted_graph_edges`:** Calcula dinamicamente o grau de cada nó e expõe `(source_id, target_id, effective_weight)` pronta para ser carregada diretamente em estruturas matriciais esparsas (CSR / CSC).

---

## 5. Escala Atual do Grafo e Atualizações Recentes

Para o detalhamento completo das melhorias recentes no motor de difusão bidirecional em Rust, resolução transmidiática (âncoras estruturais de estúdio/diretor), links de trilhas sonoras e sistema de highlight interativo no ForceGraph, consulte [Atualizações do Sistema (Novembro 2026)](./system_evolution_2026.md).

* **Arestas Semânticas:** 3.587.306 arestas ativas no DuckDB (`graph_semantic_edges`).
* **Catálogo:** >34k filmes, >20k jogos, >38k livros, >3.25M artistas e >40.4M faixas.
* **Motor:** `youmightlike-engine` em Rust (Personalized PageRank bidirecional sobre matriz CSR paralelizada com Rayon).


