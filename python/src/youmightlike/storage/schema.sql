-- ============================================================================
-- youmightlike: Esquema Canônico e Views de Projeção em Grafo (DuckDB)
-- Fontes 100% Abertas e Livres: Wikidata (CC0) + MusicBrainz + OpenLibrary
-- ============================================================================

-- ----------------------------------------------------------------------------
-- 1. DOMÍNIO: Filmes & Séries (Wikidata / TVMaze)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_movies START 1;
CREATE TABLE IF NOT EXISTS movies (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_movies'),
    wikidata_id VARCHAR UNIQUE NOT NULL, -- Ex: 'Q183611' (Blade Runner)
    imdb_id VARCHAR,
    title VARCHAR NOT NULL,
    original_title VARCHAR,
    release_year INTEGER,
    based_on_title VARCHAR,             -- Ex: 'Do Androids Dream of Electric Sheep?'
    based_on_wikidata_id VARCHAR,       -- Ex: 'Q605249' (Livro original)
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE SEQUENCE IF NOT EXISTS seq_people START 1;
CREATE TABLE IF NOT EXISTS people (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_people'),
    wikidata_id VARCHAR UNIQUE NOT NULL, -- Ex: 'Q56005' (Ridley Scott), 'Q153996' (Vangelis)
    name VARCHAR NOT NULL
);

CREATE SEQUENCE IF NOT EXISTS seq_movie_credits START 1;
CREATE TABLE IF NOT EXISTS movie_credits (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_movie_credits'),
    movie_id BIGINT NOT NULL REFERENCES movies(id),
    person_id BIGINT NOT NULL REFERENCES people(id),
    job VARCHAR NOT NULL,               -- 'Director', 'Original Music Composer', 'Screenplay'
    department VARCHAR NOT NULL         -- 'Directing', 'Sound', 'Writing'
);

CREATE SEQUENCE IF NOT EXISTS seq_keywords START 1;
CREATE TABLE IF NOT EXISTS keywords (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_keywords'),
    name VARCHAR UNIQUE NOT NULL,       -- Normalizado em minúsculas (ex: 'cyberpunk', 'dystopia')
    source VARCHAR DEFAULT 'wikidata'   -- 'wikidata', 'openlibrary', 'lastfm'
);

CREATE TABLE IF NOT EXISTS movie_keywords (
    movie_id BIGINT NOT NULL REFERENCES movies(id),
    keyword_id BIGINT NOT NULL REFERENCES keywords(id),
    PRIMARY KEY (movie_id, keyword_id)
);

CREATE SEQUENCE IF NOT EXISTS seq_genres START 1;
CREATE TABLE IF NOT EXISTS genres (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_genres'),
    name VARCHAR UNIQUE NOT NULL
);

CREATE TABLE IF NOT EXISTS movie_genres (
    movie_id BIGINT NOT NULL REFERENCES movies(id),
    genre_id BIGINT NOT NULL REFERENCES genres(id),
    PRIMARY KEY (movie_id, genre_id)
);

-- ----------------------------------------------------------------------------
-- 2. DOMÍNIO: Músicas (MusicBrainz & Last.fm)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_artists START 1;
CREATE TABLE IF NOT EXISTS music_artists (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_artists'),
    mbid VARCHAR UNIQUE,                -- MusicBrainz ID
    wikidata_id VARCHAR UNIQUE,         -- Ponte Wikidata ⇄ MusicBrainz!
    name VARCHAR NOT NULL,
    country VARCHAR(10),
    type VARCHAR(20),                   -- 'Person', 'Group'
    listeners BIGINT DEFAULT 0,
    playcount BIGINT DEFAULT 0
);

CREATE SEQUENCE IF NOT EXISTS seq_albums START 1;
CREATE TABLE IF NOT EXISTS music_albums (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_albums'),
    mbid VARCHAR UNIQUE,
    artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    title VARCHAR NOT NULL,
    release_year INTEGER,
    primary_type VARCHAR(30)            -- 'Album', 'EP', 'Soundtrack'
);

CREATE TABLE IF NOT EXISTS artist_tags (
    artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    keyword_id BIGINT NOT NULL REFERENCES keywords(id),
    weight DOUBLE DEFAULT 1.0,          -- Relevância de tag Last.fm
    PRIMARY KEY (artist_id, keyword_id)
);

CREATE TABLE IF NOT EXISTS artist_similarities (
    source_artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    target_artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    match_score DOUBLE NOT NULL,        -- Similaridade (0.0 a 1.0)
    PRIMARY KEY (source_artist_id, target_artist_id)
);

-- ----------------------------------------------------------------------------
-- 3. DOMÍNIO: Livros (OpenLibrary & Wikidata)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_books START 1;
CREATE TABLE IF NOT EXISTS books (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_books'),
    work_key VARCHAR UNIQUE,            -- Ex: '/works/OL45804W'
    wikidata_id VARCHAR UNIQUE,         -- Ex: 'Q605249'
    title VARCHAR NOT NULL,
    first_publish_year INTEGER,
    description TEXT
);

CREATE SEQUENCE IF NOT EXISTS seq_book_authors START 1;
CREATE TABLE IF NOT EXISTS book_authors (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_book_authors'),
    author_key VARCHAR UNIQUE,          -- Ex: '/authors/OL23919A'
    wikidata_id VARCHAR UNIQUE,
    name VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS book_author_links (
    book_id BIGINT NOT NULL REFERENCES books(id),
    author_id BIGINT NOT NULL REFERENCES book_authors(id),
    PRIMARY KEY (book_id, author_id)
);

CREATE TABLE IF NOT EXISTS book_subjects (
    book_id BIGINT NOT NULL REFERENCES books(id),
    keyword_id BIGINT NOT NULL REFERENCES keywords(id),
    PRIMARY KEY (book_id, keyword_id)
);

-- ============================================================================
-- 4. VIEWS DE PROJEÇÃO EM GRAFO (Consumidas pelo Motor em Rust / SciPy)
-- ============================================================================

-- View unificada de todos os Nós do Grafo
CREATE OR REPLACE VIEW v_graph_nodes AS
SELECT 
    'movie:' || CAST(id AS VARCHAR) AS node_id,
    title AS label,
    'movie' AS node_type,
    'movie' AS media_type,
    release_year AS year,
    wikidata_id AS external_id
FROM movies
UNION ALL
SELECT 
    'person:' || CAST(id AS VARCHAR),
    name,
    'person',
    NULL,
    NULL,
    wikidata_id
FROM people
UNION ALL
SELECT 
    'keyword:' || CAST(id AS VARCHAR),
    name,
    'keyword',
    NULL,
    NULL,
    NULL
FROM keywords
UNION ALL
SELECT 
    'artist:' || CAST(id AS VARCHAR),
    name,
    'artist',
    'music',
    NULL,
    COALESCE(wikidata_id, mbid)
FROM music_artists
UNION ALL
SELECT 
    'album:' || CAST(id AS VARCHAR),
    title,
    'album',
    'music',
    release_year,
    mbid
FROM music_albums
UNION ALL
SELECT 
    'book:' || CAST(id AS VARCHAR),
    title,
    'book',
    'book',
    first_publish_year,
    COALESCE(wikidata_id, work_key)
FROM books
UNION ALL
SELECT 
    'author:' || CAST(id AS VARCHAR),
    name,
    'author',
    NULL,
    NULL,
    COALESCE(wikidata_id, author_key)
FROM book_authors;

-- View unificada de todas as Arestas do Grafo (com pesos base)
CREATE OR REPLACE VIEW v_graph_edges AS
-- Filme ➔ Diretor (has_director)
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_director' AS relation_type,
    2.0 AS base_weight
FROM movie_credits WHERE job = 'Director'
UNION ALL
-- Filme ➔ Compositor da Trilha Sonora (has_composer) -> Ponte Filme ⇄ Música!
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_composer' AS relation_type,
    2.5 AS base_weight
FROM movie_credits WHERE job = 'Original Music Composer'
UNION ALL
-- Pessoa / Compositor ➔ Artista Musical (is_artist) -> Ponte Filme ⇄ Música!
SELECT 
    'person:' || CAST(p.id AS VARCHAR) AS source_id,
    'artist:' || CAST(a.id AS VARCHAR) AS target_id,
    'is_artist' AS relation_type,
    3.0 AS base_weight
FROM people p
JOIN music_artists a ON (p.wikidata_id IS NOT NULL AND p.wikidata_id = a.wikidata_id)
                     OR (LOWER(TRIM(p.name)) = LOWER(TRIM(a.name)))
UNION ALL
-- Artista ➔ Álbum (released)
SELECT 
    'artist:' || CAST(artist_id AS VARCHAR) AS source_id,
    'album:' || CAST(id AS VARCHAR) AS target_id,
    'released' AS relation_type,
    2.0 AS base_weight
FROM music_albums
UNION ALL
-- Filme ➔ Roteirista (has_writer)
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_writer' AS relation_type,
    1.5 AS base_weight
FROM movie_credits WHERE job IN ('Screenplay', 'Writer')
UNION ALL
-- Filme ➔ Keyword (has_keyword) -> Ponte Filme ⇄ Livro
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_keyword' AS relation_type,
    1.0 AS base_weight
FROM movie_keywords
UNION ALL
-- Filme ➔ Livro Original / Adaptação (based_on) -> Ponte Direta Filme ⇄ Livro!
SELECT 
    'movie:' || CAST(m.id AS VARCHAR) AS source_id,
    'book:' || CAST(b.id AS VARCHAR) AS target_id,
    'based_on' AS relation_type,
    3.0 AS base_weight
FROM movies m
JOIN books b ON (m.based_on_wikidata_id IS NOT NULL AND m.based_on_wikidata_id = b.wikidata_id)
             OR (LOWER(TRIM(m.based_on_title)) = LOWER(TRIM(b.title)))
UNION ALL
-- Artista ➔ Tag / Microgênero (has_tag)
SELECT 
    'artist:' || CAST(artist_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_tag' AS relation_type,
    weight AS base_weight
FROM artist_tags
UNION ALL
-- Artista ➔ Artista Similar (similar_to)
SELECT 
    'artist:' || CAST(source_artist_id AS VARCHAR) AS source_id,
    'artist:' || CAST(target_artist_id AS VARCHAR) AS target_id,
    'similar_to' AS relation_type,
    match_score * 2.0 AS base_weight
FROM artist_similarities
UNION ALL
-- Livro ➔ Autor (written_by)
SELECT 
    'book:' || CAST(book_id AS VARCHAR) AS source_id,
    'author:' || CAST(author_id AS VARCHAR) AS target_id,
    'written_by' AS relation_type,
    2.0 AS base_weight
FROM book_author_links
UNION ALL
-- Livro ➔ Subject / Keyword (has_subject) -> Ponte Livro ⇄ Filme
SELECT 
    'book:' || CAST(book_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_subject' AS relation_type,
    1.0 AS base_weight
FROM book_subjects;

-- View com Mitigação de Super-Nós: Ponderação Adamic-Adar / IDF
CREATE OR REPLACE VIEW v_weighted_graph_edges AS
WITH node_degrees AS (
    SELECT target_id, COUNT(*) AS degree
    FROM v_graph_edges
    GROUP BY target_id
)
SELECT 
    e.source_id,
    e.target_id,
    e.relation_type,
    ROUND(e.base_weight / LN(GREATEST(nd.degree, 2.718281828)), 4) AS effective_weight
FROM v_graph_edges e
JOIN node_degrees nd ON e.target_id = nd.target_id;
