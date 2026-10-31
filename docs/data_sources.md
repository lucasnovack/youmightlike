# 📚 Mapeamento de Fontes de Dados — youmightlike

Este documento detalha as fontes de dados gratuitas, abertas e livres de termos invasivos utilizadas para alimentar o grafo cultural heterogêneo do **`youmightlike`**.

---

## 🎬 1. Filmes & 📺 Séries: Wikidata (Wikimedia Foundation)

* **Site Oficial:** [https://www.wikidata.org/](https://www.wikidata.org/)
* **Documentação SPARQL:** [https://query.wikidata.org/](https://query.wikidata.org/)
* **Tipo de Acesso:** API REST (`wbsearchentities`) + SPARQL Endpoint (`query.wikidata.org/sparql`).
* **Licença:** **Creative Commons Zero (CC0 / Domínio Público)**.
* **Autenticação:** **Zero cadastro, zero API keys, zero dados pessoais**. Requer apenas cabeçalho `User-Agent` descritivo.
* **Rate Limits:** Livre e generoso para requisições de desenvolvimento/pessoal.

### Principais Entidades e Propriedades Extraídas:
1. **Filme / Série (`Q11424` / `Q53984`):**
   - Título, data/ano de publicação (`P577`), identificador IMDb (`P345`).
2. **Pessoas & Créditos Criativos:**
   - **Direção (`P57`):** Cria nós `person` com aresta `has_director`.
   - **Compositor da Trilha Sonora (`P86`):**
     - **Papel de Ouro:** Cria a ponte transmidiática primária entre Filmes e Músicas (ex: Vangelis, Hans Zimmer, Trent Reznor).
   - **Roteirista (`P58`):** Aresta `has_writer`.
3. **Ponte Direta com Livros:**
   - **Baseado na Obra (`P144`):** Conecta diretamente o filme ao livro/romance que o inspirou (ex: *Blade Runner* ➔ *Do Androids Dream of Electric Sheep?* de Philip K. Dick).
4. **Gêneros & Temáticas:**
   - **Gênero (`P136`):** Classificação estética (*cyberpunk*, *neo-noir*, *dystopia*).
   - **Assunto Principal (`P921`):** Temáticas semânticas (*artificial intelligence*, *space travel*).

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
