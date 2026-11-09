# Data Sources and Public Dumps

`youmightlike` was designed with an uncompromising architectural rule: **zero API subscription costs, zero registration or credential approvals, and zero fragile web scraping**.

The entire graph dataset is powered by open datasets (CC0, Public Domain, and open community dumps) processed natively from columnar formats (Parquet and gzip-compressed TSVs) directly into DuckDB.

---

## 1. Movies and People (IMDb Non-Commercial Datasets)

* **Source:** [Official IMDb Datasets](https://developer.imdb.com/non-commercial-datasets/)
* **License:** Free for personal / non-commercial use
* **Format:** Gzip-compressed TSV (`.tsv.gz`)
* **Files Used:**
  - `title.basics.tsv.gz`: Title metadata (movies, release year, runtime, primary genres).
  - `title.ratings.tsv.gz`: Average ratings and total vote counts.
  - `title.crew.tsv.gz`: Director and writer credits for each title.
  - `name.basics.tsv.gz`: Canonical personal names and identifiers (`nmXXXXXXX`).
* **Ingestion Threshold:** `movie` titles with at least 2,500 votes (statistical significance filter to exclude obscure entries without graph connectivity).
* **Ingested Volume:** ~29,200 movies, ~37,000 directors/writers, and ~100,500 creative credits.

---

## 1.1 TV Series and Mini-Series (IMDb Non-Commercial Datasets)

* **Source:** [Official IMDb Datasets](https://developer.imdb.com/non-commercial-datasets/)
* **License:** Free for personal / non-commercial use
* **Format:** Gzip-compressed TSV (`.tsv.gz`)
* **Role in Graph:** Connects serialized narrative works (*Breaking Bad, Game of Thrones, Twin Peaks, Dark, Chernobyl, Succession, The Sopranos, Peaky Blinders*), their creators/showrunners, directors, and writers (`has_director`, `has_writer`), genres, soundtrack bridges, and literary adaptations (`based_on`).
* **Ingestion Threshold:** `tvSeries` and `tvMiniSeries` titles with at least 2,500 votes.
* **Ingested Volume:** 7,647 series/mini-series, 59,308 directing/showrunning credits, 93,181 writing credits, and 18,829 genre and tag connections.

---

## 2. Movie Semantics and Tags (MovieLens Tag Genome / GroupLens)

* **Source:** [MovieLens 20M/25M Tag Genome](https://grouplens.org/datasets/movielens/) (GroupLens Research / University of Minnesota)
* **License:** Free for research and non-commercial use
* **Format:** Tabular CSVs (`genome_tags.csv`, `genome_scores.csv`, `link.csv`)
* **Role in Graph:** Links films to high-precision conceptual tags (`cyberpunk`, `dystopia`, `mindfuck`, `existentialism`, `atmospheric`, `neo-noir`, `space-opera`, `dark`, `philosophical`) with continuous relevance scores (0.0 to 1.0).
* **Link with IMDb:** Exact mapping via `link.imdbId` converted into canonical representation (`ttXXXXXXX`).
* **Ingestion Threshold:** Tags with continuous relevance score ≥ 0.65.
* **Ingested Volume:** 218,910 conceptual edges linking over 15,500 films to 1,111 refined aesthetic tags.

---

## 3. Animations, Studios, and Tropes (MyAnimeList Top Anime)

* **Source:** [MyAnimeList Top Anime Dataset](https://huggingface.co/datasets/lyfesan/myanimelist-top-anime-dataset)
* **License:** Open community dataset
* **Format:** Tabular CSV (`top_anime.csv`, 9.2 MB)
* **Role in Graph:** Links animated series, OVAs, and feature films to renowned studios (*Studio Ghibli*, *Madhouse*, *Bones*, *Wit Studio*, *Mappa*) via the `produced_by` relation and introduces narrative tropes and themes (*Time Travel*, *Psychological*, *Mecha*, *Samurai*, *Cyberpunk*).
* **Deduplication Strategy:** English and Japanese title matching against existing IMDb records. Pre-existing works receive additional credits and tags; new anime entries are ingested under the canonical key `mal:<id>`.
* **Ingestion Threshold:** Anime entries with at least 20,000 community members.
* **Ingested Volume:** 5,341 anime works integrated, 453 unique animation studios with 5,839 production edges, and 239,528 total thematic connections.

---

## 4. Video Games and Studios (Steam Community Dataset)

* **Source:** Structured Steam Store catalog ([Hugging Face CC0](https://huggingface.co/datasets/FronkonGames/steam-games-dataset))
* **License:** CC0 1.0 (Public Domain)
* **Format:** Native Parquet (`games.parquet`, 184 MB)
* **Extracted Fields:**
  - `appID`: Unique Steam App ID.
  - `name`: Official game title.
  - `release_date`: Release year.
  - `developers`: Array of game development studios (`VARCHAR[]`).
  - `genres` and `categories`: Gameplay and aesthetic tags (`VARCHAR[]`).
  - `positive` / `negative`: Steam community review counters.
* **Ingestion Threshold:** Games with at least 400 total user reviews.
* **Ingested Volume:** ~11,850 games, ~8,860 developer studios, and ~112,700 aesthetic tags.

---

## 5. Console Classics (Nintendo, PlayStation, Sega, Xbox)

* **Source:** [Video Game Sales Dataset](https://huggingface.co/datasets/Gokalp35/VideoGameSales)
* **License:** Public Domain / Open Data
* **Format:** Native Parquet (`console_games.parquet`, 1.5 MB)
* **Role in Graph:** Incorporates seminal platform exclusives unavailable on Steam (*The Legend of Zelda*, *Silent Hill*, *Super Mario*, *Bloodborne*, *Shadow of the Colossus*, *Pokémon*, classic *Metal Gear Solid*), their publishers, and platform tags.
* **Deduplication Strategy:** Normalized title join with existing Steam entries. Multiplatform titles already present are not duplicated; new console entries receive a synthetic `app_id` in a designated namespace (`2_000_000_000 + rank`).
* **Ingestion Threshold:** Games with more than 50,000 copies sold globally.
* **Ingested Volume:** 8,205 classic exclusive titles integrated (reaching 20,064 total catalog games), 9,250 developers, and 133,983 game tags.

---

## 6. Books and Authors (Goodreads 100k Dataset)

* **Source:** Open literature dataset ([Hugging Face CC0](https://huggingface.co/datasets/euclaise/goodreads_100k))
* **License:** CC0 1.0 (Public Domain)
* **Format:** Native Parquet (`goodreads_100k.parquet`, 66 MB)
* **Extracted Fields:**
  - `title`: Book title.
  - `author`: Authors (comma-separated for unnesting).
  - `desc`: Narrative synopsis / summary.
  - `genre`: Categories, microgenres, and themes.
  - `totalratings`: Total reader ratings count.
* **Ingestion Threshold:** Books with at least 300 ratings.
* **Ingested Volume:** ~38,200 books, ~29,740 authors, and ~400,200 thematic connections (`book_subjects`).

---

## 7. Music, Discographies, and Similarity Graph (MusicBrainz + Spotify + Last.fm)

* **Primary Source (Metadata & MBIDs):** [MusicBrainz Artist Dump](https://huggingface.co/datasets/LeData/media-metadata-musicbrainz-artists) (CC0 1.0)
* **Complementary Source (Albums, Tracks & Audio Features):** Spotify 114k Tracks Dataset (tabular CSV with tracks, albums, subgenres, and acoustic audio features)
* **Similarity Source (Co-listening Graph):** [Last.fm HetRec 2011 Dataset](https://grouplens.org/datasets/hetrec2011/) (GroupLens / ACM RecSys)
* **Extracted Fields:**
  - `mbid`: Canonical MusicBrainz Identifier.
  - `name`: Artist / band name.
  - `country` and `type`: Origin country and entity format (`Person` vs `Group`).
  - `tags`: Folksonomy and musical microgenres (`VARCHAR[]`).
  - `album_name`: Album titles and discographies.
  - `track_name` & `audio_features`: Individual tracks with `tempo` (BPM), `valence`, `energy`, `danceability`, `acousticness`, `instrumentalness`.
  - `user_artists`: Listener co-audition history used for cosine similarity computation with logarithmic listener weighting.
* **Ingestion Thresholds:**
  - Artists with at least 2 aesthetic characterization tags and verified names.
  - Artist similarity with at least 3 shared listeners and a cosine score ≥ 0.15.
* **Ingested Volume:** ~126,790 artists, ~63,290 albums, **85,889 individual musical tracks**, ~475,780 musical tag edges, and **61,680 bidirectional direct similarity edges** (`artist_similarities`).

---

## 8. On-Demand Connectors (Wikidata & OpenLibrary APIs)

For live queries and fine-grained enrichment of specific titles, the project maintains REST/SPARQL connectors under `python/src/youmightlike/connectors/`:
* `wikidata.py`: Extracts semantic metadata via SPARQL (directors, original soundtrack composers, literary adaptations, and cross-domain entity IDs).
* `music.py`: REST integration with MusicBrainz and Last.fm for live microgenres.
* `books.py`: Integration with the OpenLibrary API (Internet Archive) for edition metadata.

---

## 9. Open Dataset on Hugging Face (Public Parquet Store)

The complete feature store, all 27 dimensional tables, and the unified diffusion graph are publicly hosted on Hugging Face Datasets with zero-cost global CDN access:

* **Repository:** [https://huggingface.co/datasets/lnvck/youmightlike-dataset](https://huggingface.co/datasets/lnvck/youmightlike-dataset)
* **Total Volume:** **2.29 GB** (compacted from ~22 GB DuckDB using ZSTD columnar Parquet)
* **Key Contents:**
  - `music_tracks.parquet`: **40,418,287** tracks with audio features (*BPM, valence, energy, danceability*)
  - `music_albums.parquet`: **18,034,542** studio albums, EPs, and OSTs
  - `music_artists.parquet`: **3,257,918** artists, bands, and composers
  - `graph_semantic_edges.parquet`: **1,956,347** active bidirectional semantic edges
  - All cinema, anime, TV series, console games, PC titles, and literature tables.

### Remote Querying via DuckDB (HTTP Range Requests)
Researchers and developers can query any table remotely without downloading the full gigabytes:

```sql
SELECT title, popularity, tempo, energy, valence
FROM read_parquet('https://huggingface.co/datasets/lnvck/youmightlike-dataset/resolve/main/music_tracks.parquet')
WHERE popularity > 80
ORDER BY popularity DESC
LIMIT 10;
```

Export & sync scripts are maintained in:
- `scripts/export_parquets_for_hf.py`: Exports all tables from DuckDB into ZSTD Parquets and builds the Dataset Card.
- `scripts/upload_to_huggingface.py`: Synchronizes the dataset to Hugging Face Hub.
- `scripts/export_lite_db.py`: Exports `taste_graph_lite.duckdb` (140 MB) for low-memory production deployments.

