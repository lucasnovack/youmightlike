-- ============================================================================
-- youmightlike: Canonical Schema and Graph Projection Views (DuckDB)
-- 100% Open and Free Sources: Wikidata (CC0) + MusicBrainz + OpenLibrary
-- ============================================================================

-- ----------------------------------------------------------------------------
-- 1. DOMAIN: Movies & Series (Wikidata / TVMaze)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_movies START 1;
CREATE TABLE IF NOT EXISTS movies (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_movies'),
    wikidata_id VARCHAR UNIQUE,          -- E.g. 'Q183611' (Blade Runner)
    imdb_id VARCHAR UNIQUE,              -- E.g. 'tt0083658'
    title VARCHAR NOT NULL,
    original_title VARCHAR,
    release_year INTEGER,
    runtime INTEGER,
    vote_average DOUBLE,
    vote_count BIGINT,
    based_on_title VARCHAR,              -- E.g. 'Do Androids Dream of Electric Sheep?'
    based_on_wikidata_id VARCHAR,        -- E.g. 'Q605249' (Original book)
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE SEQUENCE IF NOT EXISTS seq_people START 1;
CREATE TABLE IF NOT EXISTS people (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_people'),
    wikidata_id VARCHAR UNIQUE,          -- E.g. 'Q56005' (Ridley Scott)
    imdb_id VARCHAR UNIQUE,              -- E.g. 'nm0000631'
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
    name VARCHAR UNIQUE NOT NULL,       -- Normalized lowercase (e.g. 'cyberpunk', 'dystopia')
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
-- 1.1 DOMAIN: TV Series & Mini-Series (IMDb)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_tv_series START 1;
CREATE TABLE IF NOT EXISTS tv_series (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_tv_series'),
    imdb_id VARCHAR UNIQUE,              -- E.g. 'tt0903747' (Breaking Bad)
    wikidata_id VARCHAR UNIQUE,
    title VARCHAR NOT NULL,
    original_title VARCHAR,
    series_type VARCHAR,                 -- 'tvSeries', 'tvMiniSeries'
    start_year INTEGER,
    end_year INTEGER,
    runtime INTEGER,
    vote_average DOUBLE,
    vote_count BIGINT,
    based_on_title VARCHAR,
    based_on_wikidata_id VARCHAR,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE SEQUENCE IF NOT EXISTS seq_series_credits START 1;
CREATE TABLE IF NOT EXISTS series_credits (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_series_credits'),
    series_id BIGINT NOT NULL REFERENCES tv_series(id),
    person_id BIGINT NOT NULL REFERENCES people(id),
    job VARCHAR NOT NULL,               -- 'Director', 'Writer', 'Creator', 'Original Music Composer'
    department VARCHAR NOT NULL         -- 'Directing', 'Writing', 'Sound'
);

CREATE TABLE IF NOT EXISTS series_genres (
    series_id BIGINT NOT NULL REFERENCES tv_series(id),
    genre_id BIGINT NOT NULL REFERENCES genres(id),
    PRIMARY KEY (series_id, genre_id)
);

CREATE TABLE IF NOT EXISTS series_keywords (
    series_id BIGINT NOT NULL REFERENCES tv_series(id),
    keyword_id BIGINT NOT NULL REFERENCES keywords(id),
    PRIMARY KEY (series_id, keyword_id)
);

-- ----------------------------------------------------------------------------
-- 2. DOMAIN: Music (MusicBrainz & Last.fm)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_artists START 1;
CREATE TABLE IF NOT EXISTS music_artists (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_artists'),
    mbid VARCHAR UNIQUE,                -- MusicBrainz ID
    wikidata_id VARCHAR UNIQUE,         -- Wikidata ⇄ MusicBrainz bridge!
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
    weight DOUBLE DEFAULT 1.0,          -- Last.fm tag relevance weight
    PRIMARY KEY (artist_id, keyword_id)
);

CREATE TABLE IF NOT EXISTS artist_similarities (
    source_artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    target_artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    match_score DOUBLE NOT NULL,        -- Similarity score (0.0 to 1.0)
    PRIMARY KEY (source_artist_id, target_artist_id)
);

CREATE SEQUENCE IF NOT EXISTS seq_tracks START 1;
CREATE TABLE IF NOT EXISTS music_tracks (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_tracks'),
    spotify_id VARCHAR UNIQUE,
    artist_id BIGINT NOT NULL REFERENCES music_artists(id),
    album_id BIGINT REFERENCES music_albums(id),
    title VARCHAR NOT NULL,
    duration_ms BIGINT,
    popularity INTEGER,
    danceability DOUBLE,
    energy DOUBLE,
    tempo DOUBLE,
    valence DOUBLE,
    acousticness DOUBLE,
    instrumentalness DOUBLE
);

-- ----------------------------------------------------------------------------
-- 3. DOMAIN: Books (OpenLibrary & Wikidata)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_books START 1;
CREATE TABLE IF NOT EXISTS books (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_books'),
    work_key VARCHAR UNIQUE,            -- E.g. '/works/OL45804W'
    wikidata_id VARCHAR UNIQUE,         -- E.g. 'Q605249'
    title VARCHAR NOT NULL,
    first_publish_year INTEGER,
    description TEXT
);

CREATE SEQUENCE IF NOT EXISTS seq_book_authors START 1;
CREATE TABLE IF NOT EXISTS book_authors (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_book_authors'),
    author_key VARCHAR UNIQUE,          -- E.g. '/authors/OL23919A'
    wikidata_id VARCHAR UNIQUE,
    name VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS book_author_links (
    book_id BIGINT NOT NULL REFERENCES books(id),
    author_id BIGINT NOT NULL REFERENCES book_authors(id),
    PRIMARY KEY (book_id, author_id)
);

-- ----------------------------------------------------------------------------
-- 4. DOMAIN: Games (Steam)
-- ----------------------------------------------------------------------------
CREATE SEQUENCE IF NOT EXISTS seq_games START 1;
CREATE TABLE IF NOT EXISTS games (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_games'),
    app_id BIGINT UNIQUE NOT NULL,      -- Steam App ID (e.g. 1091500)
    title VARCHAR NOT NULL,
    release_date VARCHAR,
    release_year INTEGER,
    price DOUBLE,
    positive_reviews BIGINT,
    negative_reviews BIGINT,
    developers VARCHAR,
    publishers VARCHAR,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE SEQUENCE IF NOT EXISTS seq_game_devs START 1;
CREATE TABLE IF NOT EXISTS game_developers (
    id BIGINT PRIMARY KEY DEFAULT nextval('seq_game_devs'),
    name VARCHAR UNIQUE NOT NULL
);

CREATE TABLE IF NOT EXISTS game_dev_links (
    game_id BIGINT NOT NULL REFERENCES games(id),
    developer_id BIGINT NOT NULL REFERENCES game_developers(id),
    PRIMARY KEY (game_id, developer_id)
);

CREATE TABLE IF NOT EXISTS game_tags (
    game_id BIGINT NOT NULL REFERENCES games(id),
    keyword_id BIGINT NOT NULL REFERENCES keywords(id),
    PRIMARY KEY (game_id, keyword_id)
);

-- ============================================================================
-- 5. GRAPH PROJECTION VIEWS (Consumed by Rust / SciPy Engine)
-- ============================================================================

-- Unified view of all Graph Nodes
CREATE OR REPLACE VIEW v_graph_nodes AS
SELECT 
    'movie:' || CAST(id AS VARCHAR) AS node_id,
    title AS label,
    'movie' AS node_type,
    'movie' AS media_type,
    release_year AS year,
    COALESCE(wikidata_id, imdb_id) AS external_id
FROM movies
UNION ALL
SELECT 
    'person:' || CAST(id AS VARCHAR),
    name,
    'person',
    NULL,
    NULL,
    COALESCE(wikidata_id, imdb_id)
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
    'track:' || CAST(id AS VARCHAR),
    title,
    'track',
    'music',
    NULL,
    spotify_id
FROM music_tracks
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
FROM book_authors
UNION ALL
SELECT 
    'game:' || CAST(id AS VARCHAR),
    title,
    'game',
    'game',
    release_year,
    CAST(app_id AS VARCHAR)
FROM games
UNION ALL
SELECT 
    'developer:' || CAST(id AS VARCHAR),
    name,
    'developer',
    NULL,
    NULL,
    NULL
FROM game_developers
UNION ALL
SELECT 
    'series:' || CAST(id AS VARCHAR),
    title,
    'series',
    'series',
    start_year,
    COALESCE(wikidata_id, imdb_id)
FROM tv_series;

-- Unified view of all Graph Edges (with base weights)
CREATE OR REPLACE VIEW v_graph_edges AS
-- Movie ➔ Director (has_director)
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_director' AS relation_type,
    2.0 AS base_weight
FROM movie_credits WHERE job = 'Director'
UNION ALL
-- Movie ➔ Soundtrack Composer (has_composer) -> Movie ⇄ Music Bridge!
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_composer' AS relation_type,
    2.5 AS base_weight
FROM movie_credits WHERE job = 'Original Music Composer'
UNION ALL
-- Person / Composer ➔ Musical Artist (is_artist) -> Movie ⇄ Music Bridge!
SELECT 
    'person:' || CAST(p.id AS VARCHAR) AS source_id,
    'artist:' || CAST(a.id AS VARCHAR) AS target_id,
    'is_artist' AS relation_type,
    3.0 AS base_weight
FROM people p
JOIN music_artists a ON p.wikidata_id = a.wikidata_id
WHERE p.wikidata_id IS NOT NULL
UNION ALL
SELECT 
    'person:' || CAST(p.id AS VARCHAR) AS source_id,
    'artist:' || CAST(a.id AS VARCHAR) AS target_id,
    'is_artist' AS relation_type,
    3.0 AS base_weight
FROM people p
JOIN music_artists a ON LOWER(TRIM(p.name)) = LOWER(TRIM(a.name))
WHERE p.wikidata_id IS NULL OR a.wikidata_id IS NULL
UNION ALL
-- Artist ➔ Album (released)
SELECT 
    'artist:' || CAST(artist_id AS VARCHAR) AS source_id,
    'album:' || CAST(id AS VARCHAR) AS target_id,
    'released' AS relation_type,
    2.0 AS base_weight
FROM music_albums
UNION ALL
-- Artist ➔ Track (has_track)
SELECT 
    'artist:' || CAST(artist_id AS VARCHAR) AS source_id,
    'track:' || CAST(id AS VARCHAR) AS target_id,
    'has_track' AS relation_type,
    2.0 AS base_weight
FROM music_tracks
UNION ALL
-- Album ➔ Track (contains_track)
SELECT 
    'album:' || CAST(album_id AS VARCHAR) AS source_id,
    'track:' || CAST(id AS VARCHAR) AS target_id,
    'contains_track' AS relation_type,
    1.5 AS base_weight
FROM music_tracks WHERE album_id IS NOT NULL
UNION ALL
-- Movie ➔ Screenwriter (has_writer)
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_writer' AS relation_type,
    1.5 AS base_weight
FROM movie_credits WHERE job IN ('Screenplay', 'Writer')
UNION ALL
-- Movie / Anime ➔ Animation Studio (produced_by)
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'produced_by' AS relation_type,
    2.0 AS base_weight
FROM movie_credits WHERE job = 'Animation Studio'
UNION ALL
-- Movie ➔ Keyword (has_keyword) -> Movie ⇄ Book Bridge
SELECT 
    'movie:' || CAST(movie_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_keyword' AS relation_type,
    1.0 AS base_weight
FROM movie_keywords
UNION ALL
-- Movie ➔ Original Book / Adaptation (based_on) -> Direct Movie ⇄ Book Bridge!
SELECT 
    'movie:' || CAST(m.id AS VARCHAR) AS source_id,
    'book:' || CAST(b.id AS VARCHAR) AS target_id,
    'based_on' AS relation_type,
    3.0 AS base_weight
FROM movies m
JOIN books b ON (m.based_on_wikidata_id IS NOT NULL AND m.based_on_wikidata_id = b.wikidata_id)
             OR (LOWER(TRIM(m.based_on_title)) = LOWER(TRIM(b.title)))
UNION ALL
-- Artist ➔ Tag / Microgenre (has_tag)
SELECT 
    'artist:' || CAST(artist_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_tag' AS relation_type,
    weight AS base_weight
FROM artist_tags
UNION ALL
-- Artist ➔ Similar Artist (similar_to)
SELECT 
    'artist:' || CAST(source_artist_id AS VARCHAR) AS source_id,
    'artist:' || CAST(target_artist_id AS VARCHAR) AS target_id,
    'similar_to' AS relation_type,
    match_score * 2.0 AS base_weight
FROM artist_similarities
UNION ALL
-- Book ➔ Author (written_by)
SELECT 
    'book:' || CAST(book_id AS VARCHAR) AS source_id,
    'author:' || CAST(author_id AS VARCHAR) AS target_id,
    'written_by' AS relation_type,
    2.0 AS base_weight
FROM book_author_links
UNION ALL
-- Book ➔ Subject / Keyword (has_subject) -> Book ⇄ Movie Bridge
SELECT 
    'book:' || CAST(book_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_subject' AS relation_type,
    1.0 AS base_weight
FROM book_subjects
UNION ALL
-- Game ➔ Developer (developed_by)
SELECT 
    'game:' || CAST(game_id AS VARCHAR) AS source_id,
    'developer:' || CAST(developer_id AS VARCHAR) AS target_id,
    'developed_by' AS relation_type,
    2.5 AS base_weight
FROM game_dev_links
UNION ALL
-- Game ➔ Keyword / Tag (has_tag) -> Games ⇄ Movies ⇄ Books Bridge!
SELECT 
    'game:' || CAST(game_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_tag' AS relation_type,
    1.2 AS base_weight
FROM game_tags
UNION ALL
-- Series ➔ Director (has_director)
SELECT 
    'series:' || CAST(series_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_director' AS relation_type,
    2.0 AS base_weight
FROM series_credits WHERE job = 'Director'
UNION ALL
-- Series ➔ Screenwriter / Creator (has_writer)
SELECT 
    'series:' || CAST(series_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_writer' AS relation_type,
    1.8 AS base_weight
FROM series_credits WHERE job IN ('Writer', 'Creator', 'Screenplay')
UNION ALL
-- Series ➔ Soundtrack Composer (has_composer)
SELECT 
    'series:' || CAST(series_id AS VARCHAR) AS source_id,
    'person:' || CAST(person_id AS VARCHAR) AS target_id,
    'has_composer' AS relation_type,
    2.5 AS base_weight
FROM series_credits WHERE job = 'Original Music Composer'
UNION ALL
-- Series ➔ Keyword / Theme (has_keyword)
SELECT 
    'series:' || CAST(series_id AS VARCHAR) AS source_id,
    'keyword:' || CAST(keyword_id AS VARCHAR) AS target_id,
    'has_keyword' AS relation_type,
    1.0 AS base_weight
FROM series_keywords
UNION ALL
-- Series ➔ Original Book / Adaptation (based_on)
SELECT 
    'series:' || CAST(s.id AS VARCHAR) AS source_id,
    'book:' || CAST(b.id AS VARCHAR) AS target_id,
    'based_on' AS relation_type,
    3.0 AS base_weight
FROM tv_series s
JOIN books b ON (s.based_on_wikidata_id IS NOT NULL AND s.based_on_wikidata_id = b.wikidata_id)
             OR (LOWER(TRIM(s.based_on_title)) = LOWER(TRIM(b.title)))
UNION ALL
-- =========================================================================
-- HISTORICAL CONTEXT AND EPOCH BRIDGES (released_in_epoch)
-- =========================================================================
-- Games by year
SELECT 
    'game:' || CAST(id AS VARCHAR) AS source_id,
    'epoch:' || CAST(release_year AS VARCHAR) AS target_id,
    'released_in_epoch' AS relation_type,
    0.8 AS base_weight
FROM games WHERE release_year BETWEEN 1970 AND 2026
UNION ALL
-- Movies by year
SELECT 
    'movie:' || CAST(id AS VARCHAR) AS source_id,
    'epoch:' || CAST(release_year AS VARCHAR) AS target_id,
    'released_in_epoch' AS relation_type,
    0.8 AS base_weight
FROM movies WHERE release_year BETWEEN 1970 AND 2026
UNION ALL
-- TV Series by year
SELECT 
    'series:' || CAST(id AS VARCHAR) AS source_id,
    'epoch:' || CAST(start_year AS VARCHAR) AS target_id,
    'released_in_epoch' AS relation_type,
    0.8 AS base_weight
FROM tv_series WHERE start_year BETWEEN 1970 AND 2026;

-- Supernode Mitigation View: Adamic-Adar / IDF Weighting
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
