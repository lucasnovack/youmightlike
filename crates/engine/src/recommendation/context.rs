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
    pub top_diffused_artist_ids: &'a [i64],
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

/// Filter list of generic Steam infrastructure, accessibility, hardware platform, and nationality metadata tags
pub fn is_steam_noise_tag(tag: &str) -> bool {
    if tag.starts_with("console-") {
        return true;
    }
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
            | "american"
            | "united states"
            | "usa"
            | "u.s.a."
            | "british"
            | "uk"
            | "united kingdom"
            | "english"
            | "england"
            | "canadian"
            | "canada"
            | "australian"
            | "australia"
            | "german"
            | "germany"
            | "french"
            | "france"
            | "japanese"
            | "japan"
            | "swedish"
            | "sweden"
            | "polish"
            | "poland"
            | "european"
    )
}

/// Normalizes game title by stripping trademark symbols, year suffixes, and edition variants
/// (e.g. GOTY, Remastered, VR, Special Edition) for robust franchise deduplication.
pub fn normalize_game_title(title: &str) -> String {
    let mut s = title.to_lowercase();
    s.retain(|c| c != '®' && c != '™' && c != '©');

    if let Some(idx) = s.find('(') {
        if let Some(end_idx) = s[idx..].find(')') {
            let inner = &s[idx + 1..idx + end_idx];
            if inner.len() == 4 && inner.chars().all(|c| c.is_ascii_digit()) {
                s.replace_range(idx..=idx + end_idx, "");
            }
        }
    }

    let patterns = [
        "game of the year edition",
        "game of the year",
        "goty edition",
        "goty",
        "special edition",
        "deluxe edition",
        "deluxe",
        "remastered",
        "remaster",
        "definitive edition",
        "enhanced edition",
        "anniversary edition",
        "director's cut",
        "vr",
        "add-on pack",
    ];
    for pat in patterns {
        s = s.replace(pat, " ");
    }

    // Truncate generic subtitle / expansion packs separated by " - " (e.g. "Game - Subtitle")
    if let Some(pos) = s.find(" - ") {
        if s[..pos].trim().len() >= 3 {
            s.truncate(pos);
        }
    }

    let mut cleaned = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            cleaned.push(c);
            prev_space = false;
        } else if !prev_space {
            cleaned.push(' ');
            prev_space = true;
        }
    }
    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_game_title() {
        assert_eq!(
            normalize_game_title("The Elder Scrolls V: Skyrim"),
            "the elder scrolls v skyrim"
        );
        assert_eq!(
            normalize_game_title("The Elder Scrolls V: Skyrim Special Edition"),
            "the elder scrolls v skyrim"
        );
        assert_eq!(
            normalize_game_title("The Elder Scrolls V: Skyrim VR"),
            "the elder scrolls v skyrim"
        );
        assert_eq!(
            normalize_game_title("The Elder Scrolls IV: Oblivion Remastered"),
            "the elder scrolls iv oblivion"
        );
        assert_eq!(
            normalize_game_title(
                "The Elder Scrolls IV: Oblivion® Game of the Year Edition Deluxe (2009)"
            ),
            "the elder scrolls iv oblivion"
        );
        assert_eq!(
            normalize_game_title("Fallout 4"),
            "fallout 4"
        );
        assert_eq!(
            normalize_game_title("Fallout 4 VR"),
            "fallout 4"
        );
        assert_eq!(
            normalize_game_title("Far Cry®"),
            "far cry"
        );
        assert_eq!(
            normalize_game_title("Far Cry 3"),
            "far cry 3"
        );
        assert_ne!(
            normalize_game_title("Far Cry"),
            normalize_game_title("Far Cry 3")
        );
    }
}
