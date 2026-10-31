# 📚 Mapeamento de Fontes de Dados — youmightlike

Este documento detalha as fontes de dados gratuitas e abertas utilizadas para alimentar o grafo cultural heterogêneo do **`youmightlike`**.

---

## 🎬 1. Filmes & 📺 Séries: TMDB (The Movie Database)

* **Site Oficial:** [https://www.themoviedb.org/](https://www.themoviedb.org/)
* **Documentação da API:** [https://developer.themoviedb.org/docs](https://developer.themoviedb.org/docs)
* **Tipo de Acesso:** API REST v3/v4 gratuita (requer criação de conta e geração de chave de API).
* **Custo:** R$ 0,00 (Gratuito para uso não-comercial/desenvolvimento).
* **Rate Limits:** ~40 a 50 requisições a cada 10 segundos.

### Dumps Diários de IDs (Daily File Exports)
O TMDB fornece exportações diárias compactadas em formato JSON Lines contendo todos os IDs válidos de obras e pessoas.
* URL Base: `http://files.tmdb.org/p/exports/movie_ids_MM_DD_YYYY.json.gz`
* Tipos exportados: `movie_ids`, `tv_series_ids`, `person_ids`, `keyword_ids`.

### Principais Endpoints & Entidades Extraídas:
1. **`/movie/{movie_id}` e `/tv/{series_id}`:**
   - Título, título original, sinopse, tagline.
   - Ano/data de lançamento (`release_date`), duração (`runtime`).
   - Avaliação (`vote_average`), contagem de votos (`vote_count`), popularidade.
   - Gêneros (`genres`).
2. **`/movie/{movie_id}/keywords`:**
   - Palavras-chave semânticas e temáticas (ex: *cyberpunk*, *neo-noir*, *dystopia*, *artificial intelligence*).
   - **Papel:** Ponte estética direta com livros (OpenLibrary) e games (Steam).
3. **`/movie/{movie_id}/credits`:**
   - Direção (`crew` onde `job = 'Director'`).
   - Roteiro (`crew` onde `job = 'Screenplay'` ou `'Writer'`).
   - **Trilha Sonora Original (`crew` onde `job = 'Original Music Composer'`):**
     - **Papel de Ouro:** Cria a ponte transmidiática primária entre Filmes e Músicas (ex: Hans Zimmer, Vangelis, Trent Reznor).
   - Direção de Fotografia (`crew` onde `job = 'Director of Photography'`).
   - Elenco principal (`cast`).

---

## 🎵 2. Músicas: MusicBrainz + Last.fm

### Fonte A: MusicBrainz (MetaBrainz Foundation)
* **Site Oficial:** [https://musicbrainz.org/](https://musicbrainz.org/)
* **Tipo de Acesso:** API REST pública aberta ou dumps completos (PostgreSQL/JSON).
* **Custo:** R$ 0,00 (Sem fins lucrativos / Creative Commons).
* **Autenticação:** Não requer chave de API; exige apenas um cabeçalho `User-Agent` descritivo.
* **Rate Limit:** 1 requisição/segundo na API pública.
* **Entidades Extraídas:**
  - `Artist`: Nome, país, tipo (grupo, pessoa), datas de atividade.
  - `Release Group` (Álbuns / EPs): Título, data da primeira versão, tipo primário.
  - `Work` / `Recording`: Faixas, compositores e letristas.
  - `Relationships`: Relações entre artistas (membro de banda, projetos paralelos, colaborações).

### Fonte B: Last.fm API
* **Site Oficial:** [https://www.last.fm/api](https://www.last.fm/api)
* **Tipo de Acesso:** API REST pública (requer API Key gratuita).
* **Custo:** R$ 0,00.
* **Entidades Extraídas:**
  - **Folksonomia de Tags Comunitárias (`artist.getTopTags`):** Microgêneros e humores curados por ouvintes há mais de 20 anos (*shoegaze*, *post-punk*, *dark ambient*, *melancholic*, *synthwave*).
  - **Similaridade Pré-calculada (`artist.getSimilar`):** Artistas com proximidade de escuta e respectivos índices de afinidade.
  - Principais faixas e álbuns do artista (`artist.getTopTracks`, `artist.getTopAlbums`).

---

## 📚 3. Livros: Open Library (Internet Archive)

* **Site Oficial:** [https://openlibrary.org/](https://openlibrary.org/)
* **Documentação:** [https://openlibrary.org/developers/api](https://openlibrary.org/developers/api)
* **Tipo de Acesso:** API REST aberta + Dumps mensais em TSV/JSON.
* **Custo:** R$ 0,00 (Projeto sem fins lucrativos).
* **Autenticação:** Nenhuma necessária.
* **Entidades Extraídas:**
  - `Works`: Título, sinopse, ano da primeira publicação.
  - `Authors`: Autores e biografias.
  - **`Subjects` (Assuntos / Temas):** Classificações temáticas (ex: *cyberpunk fiction*, *existentialism*, *space exploration*, *dystopian fiction*).
  - **Papel:** Conecta diretamente com as *keywords* do TMDB e tags do Last.fm.

---

## 🎮 4. Games (Fase de Expansão): IGDB & Steam API

* **IGDB (Twitch Developer):** API gratuita para desenvolvedores com dados profundos de desenvolvedoras, franquias, temas e mecânicas de gameplay.
* **Steam Web API:** Acesso público a metadados de jogos e tags comunitárias da Steam store.
