use crate::model::{PlaylistKind, Relation, Team};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    SmurfSuspect,
    OneVOneSpecialist,
    DefensiveAnchor,
    OnTilt,
    PeakDisparity,
}

impl Badge {
    pub fn label_fr(self) -> &'static str {
        match self {
            Badge::SmurfSuspect => "Potentiel smurf",
            Badge::OneVOneSpecialist => "Joueur 1v1",
            Badge::DefensiveAnchor => "Gardien",
            Badge::OnTilt => "Lose streak",
            Badge::PeakDisparity => "Ancien haut rang",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStatus {
    Loading,
    Ready,
    Failed,
}

/// Pourquoi un rang est affiché : seuls les rangs utiles sont montrés.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankRole {
    /// Rang dans le mode de la partie en cours.
    Mode,
    /// Rang actuel d'un ancien haut niveau.
    Current,
    /// Rang maximum atteint.
    Peak,
    /// Autre rang qui explique un badge (1v1 d'un spécialiste, meilleur rang d'un smurf).
    Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankView {
    pub role: RankRole,
    pub playlist: PlaylistKind,
    pub label: String,
    pub tier: u32,
    pub tier_name: String,
}

/// Matchs déjà joués avec ou contre ce joueur (résultats vus de mon côté).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterView {
    pub against_wins: u32,
    pub against_losses: u32,
    pub with_wins: u32,
    pub with_losses: u32,
    /// Heure Unix du dernier match partagé.
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerView {
    pub key: String,
    pub name: String,
    pub team: Team,
    pub relation: Relation,
    pub status: PlayerStatus,
    /// 0-100, 0 tant que le profil n'est pas chargé.
    pub smurf_index: u32,
    pub badges: Vec<Badge>,
    pub badge_labels: Vec<String>,
    pub ranks: Vec<RankView>,
    pub best_tier_name: String,
    pub total_wins: Option<u32>,
    /// Historique des matchs déjà joués avec ou contre lui, s'il y en a.
    #[serde(default)]
    pub encounter: Option<EncounterView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LobbySnapshot {
    pub players: Vec<PlayerView>,
    /// Nombre de joueurs encore en cours de résolution.
    pub pending: usize,
    pub updated_at: i64,
    /// Mode de la partie en cours (1v1, 2v2, 3v3…), s'il est connu.
    #[serde(default)]
    pub playlist: Option<PlaylistKind>,
    /// Joueur local détecté ou configuré (ex. « neyor. · epic »).
    #[serde(default)]
    pub me: Option<String>,
}
