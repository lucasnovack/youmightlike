# Fontes de Dados e Dumps Utilizados

O `youmightlike` foi desenhado com uma regra clara: **zero custo de API, zero dependência de aprovação cadastral e zero scraping frágil**.

Toda a malha de dados é alimentada por datasets abertos (CC0 / Domínio Público / dumps comunitários) processados nativamente em arquivos colunares (Parquet e TSV compactados em gzip) direto para o DuckDB.

---

## 1. Filmes e Pessoas (IMDb Non-Commercial Datasets)

* **Origem:** [IMDb Datasets oficiais](https://developer.imdb.com/non-commercial-datasets/)
* **Licença:** Gratuito para uso pessoal / não-comercial
* **Formato:** TSV compactado (`.tsv.gz`)
* **Arquivos utilizados:**
  - `title.basics.tsv.gz`: Metadados dos títulos (filmes, ano de lançamento, duração, gêneros primários).
  - `title.ratings.tsv.gz`: Avaliações médias e contagem de votos.
  - `title.crew.tsv.gz`: Mapeamento de diretores e roteiristas de cada título.
  - `name.basics.tsv.gz`: Nomes e identificadores canônicos de pessoas (`nmXXXXXXX`).
* **Filtro de Ingestão:** Títulos do tipo `movie` com pelo menos 2.500 votos (corte de relevância estatística para evitar títulos obscuros sem conexões).
* **Volume Ingerido:** ~29.200 filmes, ~37.000 diretores/roteiristas e ~100.500 créditos criativos.

---

## 2. Jogos e Estúdios (Steam Community Dataset)

* **Origem:** Dataset estruturado da loja Steam ([Hugging Face CC0](https://huggingface.co/datasets/FronkonGames/steam-games-dataset))
* **Licença:** CC0 (Domínio Público)
* **Formato:** Parquet nativo (`games.parquet`, 184 MB)
* **Campos Extraídos:**
  - `appID`: Steam App ID único.
  - `name`: Título oficial do jogo.
  - `release_date`: Ano de lançamento.
  - `developers`: Array de estúdios desenvolvedores (`VARCHAR[]`).
  - `genres` e `categories`: Tags de gameplay e estéticas (`VARCHAR[]`).
  - `positive` / `negative`: Reviews da comunidade na Steam.
* **Filtro de Ingestão:** Jogos com no mínimo 400 reviews totais.
* **Volume Ingerido:** ~11.850 jogos, ~8.860 desenvolvedoras e ~112.700 tags estéticas.

---

## 3. Livros e Autores (Goodreads 100k Dataset)

* **Origem:** Dataset aberto de literatura ([Hugging Face CC0](https://huggingface.co/datasets/euclaise/goodreads_100k))
* **Licença:** CC0 1.0 (Public Domain)
* **Formato:** Parquet nativo (`goodreads_100k.parquet`, 66 MB)
* **Campos Extraídos:**
  - `title`: Título do livro.
  - `author`: Autores (separados por vírgula para desaninhamento).
  - `desc`: Sinopse / resumo da obra.
  - `genre`: Categorias, microgêneros e temas.
  - `totalratings`: Quantidade de avaliações dos leitores.
* **Filtro de Ingestão:** Obras com no mínimo 300 avaliações.
* **Volume Ingerido:** ~38.200 livros, ~29.740 autores e ~400.200 conexões temáticas (`book_subjects`).

---

## 4. Música e Discografias (MusicBrainz + Spotify Tracks)

* **Origem Primária:** [MusicBrainz Artist Dump](https://huggingface.co/datasets/LeData/media-metadata-musicbrainz-artists) (CC0 1.0)
* **Origem Complementar:** Spotify 114k Tracks Dataset (CSV tabular com faixas, álbuns e subgêneros)
* **Campos Extraídos:**
  - `mbid`: MusicBrainz Identifier canônico.
  - `name`: Nome do artista / banda.
  - `country` e `type`: País de origem e formato (`Person` vs `Group`).
  - `tags`: Folksonomia e microgêneros musicais (`VARCHAR[]`).
  - `album_name`: Títulos de álbuns e discografias.
* **Filtro de Ingestão:** Artistas com pelo menos 2 tags de caracterização estética e nomes reais (descartando entradas dummy como `[unknown]` ou `Various Artists`).
* **Volume Ingerido:** ~126.790 artistas, ~63.290 álbuns e ~475.780 arestas de tags musicais.

---

## 5. Conectores Pontuais (Wikidata & OpenLibrary APIs)

Para pesquisas sob demanda ou enriquecimento de obras específicas, o projeto possui conectores REST/SPARQL em `python/src/youmightlike/connectors/`:
* `wikidata.py`: Extrai metadados semânticos via SPARQL (direção, compositores de trilha sonora, adaptações literárias e vínculos de IDs cruzados).
* `music.py`: Integração com API REST do MusicBrainz e Last.fm para microgêneros ao vivo.
* `books.py`: Integração com OpenLibrary API (Internet Archive) para metadados de edições.
