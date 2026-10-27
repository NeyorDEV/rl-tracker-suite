//! Réglages de LobbyTracker. Ils sont stockés dans la section `lobbytracker` du fichier de
//! paramètres de RL Tracker Suite (un seul fichier, une seule interface de sauvegarde).
//! La Stats API, le chemin du jeu et le serveur OBS sont ceux de RL Tracker Suite : ils ne sont
//! pas répétés ici.

use serde::{Deserialize, Serialize};

/// Cases à cocher de l'écran de paramètres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub smurf_suspect: bool,
    pub one_v_one_specialist: bool,
    pub defensive_anchor: bool,
    pub on_tilt: bool,
    pub peak_disparity: bool,
    pub only_opponents: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            smurf_suspect: true,
            one_v_one_specialist: true,
            defensive_anchor: true,
            on_tilt: true,
            peak_disparity: true,
            only_opponents: false,
        }
    }
}

/// Disposition de l'overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverlayLayout {
    /// Lignes compactes empilées.
    Column,
    /// Bande horizontale de puces compactes.
    Row,
    /// Cartes empilées verticalement.
    #[default]
    CardVertical,
    /// Cartes côte à côte.
    CardHorizontal,
}

/// Couleurs de l'overlay, au format `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayColors {
    pub background: String,
    pub text: String,
    pub muted: String,
    /// Liseré des adversaires, des coéquipiers et des joueurs d'équipe inconnue.
    pub opponent: String,
    pub teammate: String,
    pub unknown: String,
    pub badge_smurf: String,
    pub badge_one_v_one: String,
    pub badge_anchor: String,
    pub badge_tilt: String,
    pub badge_peak: String,
}

impl Default for OverlayColors {
    fn default() -> Self {
        let s = |v: &str| v.to_string();
        Self {
            background: s("#0c0e14"),
            text: s("#ffffff"),
            muted: s("#9aa4b8"),
            opponent: s("#ff7a2b"),
            teammate: s("#3b9dff"),
            unknown: s("#888888"),
            badge_smurf: s("#e03a3a"),
            badge_one_v_one: s("#7b2cff"),
            badge_anchor: s("#7b2cff"),
            badge_tilt: s("#6b7280"),
            badge_peak: s("#b7791f"),
        }
    }
}

/// Quand l'overlay se masque tout seul après l'arrivée d'un lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoHideMode {
    /// Jamais : l'overlay reste affiché.
    #[default]
    Off,
    /// Dès la fin du décompte 3-2-1 (événement `RoundStarted` de la Stats API).
    Kickoff,
    /// Après `auto_hide_seconds` secondes.
    Timer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub filters: Filters,
    /// Affiche l'overlay du lobby (raccourci global Ctrl+Maj+L).
    pub overlay_enabled: bool,
    pub overlay_layout: OverlayLayout,
    /// Échelle de l'overlay en pourcentage (50 à 200).
    pub overlay_scale: u32,
    /// Opacité du fond des cartes, en pourcentage (10 à 100).
    pub overlay_opacity: u32,
    pub overlay_colors: OverlayColors,
    /// Emplacement : `top-left`, `top-center`, `top-right`, `bottom-left`, `bottom-center`,
    /// `bottom-right` ou `custom` (même convention que l'overlay de session).
    pub overlay_position: String,
    /// Marges par rapport au bord de l'écran, en pixels logiques.
    pub overlay_offset_x: f64,
    pub overlay_offset_y: f64,
    pub overlay_custom_x: Option<f64>,
    pub overlay_custom_y: Option<f64>,
    pub auto_hide_mode: AutoHideMode,
    pub auto_hide_seconds: u32,
    /// Jeton API officiel Tracker Network (fallback).
    pub trn_api_key: Option<String>,
    /// Utiliser la WebView2 invisible pour contourner Cloudflare.
    pub use_webview_scraper: bool,
    pub cache_ttl_hours: u64,
    /// Seuil de victoires totales sous lequel un haut rang est suspect.
    pub smurf_max_wins: u32,
    /// Tier minimal (17 = Champion 2) pour l'alerte smurf.
    pub smurf_min_tier: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            filters: Filters::default(),
            overlay_enabled: true,
            overlay_layout: OverlayLayout::default(),
            overlay_scale: 100,
            overlay_opacity: 82,
            overlay_colors: OverlayColors::default(),
            overlay_position: "top-right".into(),
            overlay_offset_x: 24.0,
            overlay_offset_y: 24.0,
            overlay_custom_x: None,
            overlay_custom_y: None,
            auto_hide_mode: AutoHideMode::Off,
            auto_hide_seconds: 30,
            trn_api_key: None,
            use_webview_scraper: true,
            cache_ttl_hours: 24,
            smurf_max_wins: 250,
            smurf_min_tier: crate::model::TIER_C2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_falls_back_to_defaults() {
        let c: Config = serde_json::from_str(r#"{"filters":{"on_tilt":false}}"#).unwrap();
        assert!(!c.filters.on_tilt);
        assert!(c.filters.smurf_suspect);
        assert_eq!(c.overlay_position, "top-right");
    }

    #[test]
    fn overlay_layout_roundtrips_as_snake_case() {
        let c: Config = serde_json::from_str(r#"{"overlay_layout":"card_horizontal","overlay_scale":150}"#).unwrap();
        assert_eq!(c.overlay_layout, OverlayLayout::CardHorizontal);
        assert_eq!(c.overlay_scale, 150);
        assert!(serde_json::to_string(&c).unwrap().contains("\"card_horizontal\""));
    }

    #[test]
    fn auto_hide_mode_is_snake_case() {
        let c: Config = serde_json::from_str(r#"{"auto_hide_mode":"kickoff"}"#).unwrap();
        assert_eq!(c.auto_hide_mode, AutoHideMode::Kickoff);
    }
}
