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

## 2. Semântica e Tags de Filmes (MovieLens Tag Genome / GroupLens)

* **Origem:** [MovieLens 20M/25M Tag Genome](https://grouplens.org/datasets/movielens/) (GroupLens Research / Universidade de Minnesota)
* **Licença:** Gratuito para pesquisa e uso não-comercial
* **Formato:** CSVs tabulares (`genome_tags.csv`, `genome_scores.csv`, `link.csv`)
* **Papel no Grafo:** Conecta filmes a tags conceituais de alta precisão (`cyberpunk`, `dystopia`, `mindfuck`, `existentialism`, `atmospheric`, `neo-noir`, `space-opera`, `dark`, `philosophical`) com pontuação de relevância contínua (0.0 a 1.0).
* **Vínculo com IMDb:** Mapeamento exato via `link.imdbId` convertido em formato canônico (`ttXXXXXXX`).
* **Filtro de Ingestão:** Tags com relevância $\ge 0.65$.
* **Volume Ingerido:** 218.910 arestas conceituais conectando mais de 15.500 filmes a 1.111 tags estéticas refinadas.

---

## 3. Animações, Estúdios e Tropos (MyAnimeList Top Anime)

* **Origem:** [MyAnimeList Top Anime Dataset](https://huggingface.co/datasets/lyfesan/myanimelist-top-anime-dataset)
* **Licença:** Dados abertos de comunidade
* **Formato:** CSV tabular (`top_anime.csv`, 9.2 MB)
* **Papel no Grafo:** Conecta séries, OVAs e filmes de animação a estúdios renomados (*Studio Ghibli*, *Madhouse*, *Bones*, *Wit Studio*, *Mappa*) através da relação `produced_by` e introduz tropos/temas narrativos (*Time Travel*, *Psychological*, *Mecha*, *Samurai*, *Cyberpunk*).
* **Prevenção de Duplicatas:** Casamento de títulos em inglês e japonês com obras já catalogadas pelo IMDb. Obras pré-existentes apenas recebem novos créditos e tags; obras inéditas entram com chave `mal:<id>`.
* **Filtro de Ingestão:** Obras com pelo menos 20.000 membros na comunidade.
* **Volume Ingerido:** 5.341 obras de anime integradas, 453 estúdios de animação únicos com 5.839 arestas de produção e 239.528 conexões temáticas totais.

---

## 4. Jogos e Estúdios (Steam Community Dataset)

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

## 5. Clássicos de Console (Nintendo, PlayStation, Sega, Xbox)

* **Origem:** [Video Game Sales Dataset](https://huggingface.co/datasets/Gokalp35/VideoGameSales)
* **Licença:** Domínio Público / Dados Abertos
* **Formato:** Parquet nativo (`console_games.parquet`, 1.5 MB)
* **Papel no Grafo:** Incorpora clássicos fundamentais que não existem na Steam por exclusividade de plataforma (*The Legend of Zelda*, *Silent Hill*, *Super Mario*, *Bloodborne*, *Shadow of the Colossus*, *Pokémon*, *Metal Gear Solid clássico*), suas publicadoras e tags de plataforma.
* **Prevenção de Duplicatas:** Join de título normalizado com os jogos existentes da Steam. Títulos multiplataforma pré-existentes não são duplicados; títulos inéditos recebem `app_id` sintético no espaço reservado (`2_000_000_000 + rank`).
* **Filtro de Ingestão:** Obras com mais de 50.000 cópias vendidas globalmente.
* **Volume Ingerido:** 8.205 clássicos inéditos integrados (totalizando 20.064 jogos no catálogo), 9.250 desenvolvedoras e 133.983 tags de jogos.

---

## 6. Livros e Autores (Goodreads 100k Dataset)

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

## 7. Música, Discografias e Grafo de Similaridade (MusicBrainz + Spotify + Last.fm)

* **Origem Primária (Metadados & MBIDs):** [MusicBrainz Artist Dump](https://huggingface.co/datasets/LeData/media-metadata-musicbrainz-artists) (CC0 1.0)
* **Origem Complementar (Álbuns, Faixas & Audio Features):** Spotify 114k Tracks Dataset (CSV tabular com faixas, álbuns, subgêneros e métricas acústicas de áudio)
* **Origem de Similaridade (Co-listening Graph):** [Last.fm HetRec 2011 Dataset](https://grouplens.org/datasets/hetrec2011/) (GroupLens / ACM RecSys)
* **Campos Extraídos:**
  - `mbid`: MusicBrainz Identifier canônico.
  - `name`: Nome do artista / banda.
  - `country` e `type`: País de origem e formato (`Person` vs `Group`).
  - `tags`: Folksonomia e microgêneros musicais (`VARCHAR[]`).
  - `album_name`: Títulos de álbuns e discografias.
  - `track_name` & `audio_features`: Faixas individuais com `tempo` (BPM), `valence`, `energy`, `danceability`, `acousticness`, `instrumentalness`.
  - `user_artists`: Histórico de co-escuta de ouvintes para cálculo vetorial de similaridade de cosseno com log-weight.
* **Filtros de Ingestão:** 
  - Artistas com pelo menos 2 tags de caracterização estética e nomes reais.
  - Similaridade entre artistas com no mínimo 3 ouvintes compartilhados e índice de cosseno $\ge 0.15$.
* **Volume Ingerido:** ~126.790 artistas, ~63.290 álbuns, **85.889 faixas musicais individuais**, ~475.780 arestas de tags musicais e **61.680 arestas bidirecionais de similaridade direta** (`artist_similarities`).

---

## 8. Conectores Pontuais (Wikidata & OpenLibrary APIs)

Para pesquisas sob demanda ou enriquecimento de obras específicas, o projeto possui conectores REST/SPARQL em `python/src/youmightlike/connectors/`:
* `wikidata.py`: Extrai metadados semânticos via SPARQL (direção, compositores de trilha sonora, adaptações literárias e vínculos de IDs cruzados).
* `music.py`: Integração com API REST do MusicBrainz e Last.fm para microgêneros ao vivo.
* `books.py`: Integração com OpenLibrary API (Internet Archive) para metadados de edições.
