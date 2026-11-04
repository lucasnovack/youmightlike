//! Recommendation context, domain parameters, and shared helpers.

use crate::models::AppState;
use duckdb::Connection;
use std::collections::{HashMap, HashSet};

/// Shared execution context passed to all domain extractors
pub struct RecContext<'a> {
    pub state: &'a AppState,
    pub conn: &'a Connection,
    pub p: &'a [f32],
    pub exclude_ids: &'a HashSet<u32>,
    pub seed_set: &'a HashSet<String>,
    pub active_seed_set: &'a HashSet<String>,
    pub affinitive_artist_keys: &'a HashSet<String>,
    pub seed_artist_names: &'a HashMap<String, String>,
    pub seed_tag_counts: &'a HashMap<i64, usize>,
    pub seed_people: &'a HashSet<i64>,
    pub seed_devs: &'a HashSet<i64>,
    pub seed_authors: &'a HashSet<i64>,
    pub seed_artists: &'a HashSet<i64>,
    pub seed_games: &'a HashSet<i64>,
    pub is_pt: bool,
    pub min_rec_score: f32,
}

pub const MIN_REC_SCORE: f32 = 5e-5;

/// Filter list of generic Steam infrastructure and accessibility tags
pub fn is_steam_noise_tag(tag: &str) -> bool {
    matches!(
        tag,
        "adjustable difficulty"
            | "adjustable text size"
            | "camera comfort"
            | "captions available"
            | "chat speech-to-text"
            | "chat text-to-speech"
            | "color alternatives"
            | "commentary available"
            | "cross-platform multiplayer"
            | "custom volume controls"
            | "family sharing"
            | "full controller support"
            | "hdr available"
            | "in-app purchases"
            | "includes source sdk"
            | "includes level editor"
            | "keyboard only option"
            | "lan co-op"
            | "lan pvp"
            | "mods"
            | "mods (require hl2)"
            | "mouse only option"
            | "narrated game menus"
            | "partial controller support"
            | "playable without timed input"
            | "remote play together"
            | "remote play on phone"
            | "remote play on tv"
            | "remote play on tablet"
            | "save anytime"
            | "shared/split screen"
            | "shared/split screen co-op"
            | "shared/split screen pvp"
            | "single-player"
            | "multi-player"
            | "multiplayer"
            | "stats"
            | "steam achievements"
            | "steam cloud"
            | "steam leaderboards"
            | "steam timeline"
            | "steam trading cards"
            | "steam turn notifications"
            | "steam workshop"
            | "steamvr collectibles"
            | "stereo sound"
            | "subtitle options"
            | "surround sound"
            | "touch only option"
            | "tracked controller support"
            | "vr only"
            | "vr support"
            | "vr supported"
            | "valve anti-cheat enabled"
    )
}
