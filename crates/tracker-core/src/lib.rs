//! Types partagés par tous les crates LobbyTracker : identifiants, profils,
//! configuration utilisateur et vues envoyées à l'overlay / OBS.

pub mod config;
pub mod encounters;
pub mod model;
pub mod view;

pub use encounters::{Encounter, EncounterBook, Participant, Tally};

pub use config::{AutoHideMode, Config, Filters, OverlayColors, OverlayLayout};
pub use model::{ RecentForm,
    tier_name, PlayerId, PlayerProfile, Platform, PlaylistKind, PlaylistStat, Relation, Team,
    TIER_C2,
};
pub use view::{Badge, EncounterView, LobbySnapshot, PlayerStatus, PlayerView, RankRole, RankView};

use std::time::{SystemTime, UNIX_EPOCH};

/// Secondes Unix actuelles.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
