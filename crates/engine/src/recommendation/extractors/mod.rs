//! Domain-specific recommendation extractors.

pub mod backfill;
pub mod books;
pub mod games;
pub mod movies;
pub mod music;
pub mod series;

pub use backfill::extract_backfill;
pub use books::extract_books;
pub use games::extract_games;
pub use movies::extract_movies;
pub use music::extract_music;
pub use series::extract_series;
