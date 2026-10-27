//! Lecture des événements de la Stats API officielle : joueurs du lobby (identifiant, pseudo,
//! équipe), début de match et coup d'envoi.
//!
//! La connexion TCP, le découpage du flux JSON et le décodage `Event` / `Data` sont faits par
//! `stats_client` de RL Tracker Suite ; ce crate ne reçoit que (nom d'événement, données).

use serde_json::Value;
use tracker_core::{Platform, PlayerId, Team};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedEvent {
    PlayerSeen { id: PlayerId, name: Option<String>, team: Option<Team> },
    /// Nouveau match / retour au menu : la liste des joueurs doit être vidée.
    LobbyReset,
    /// Fin du décompte 3-2-1 : le coup d'envoi vient d'être donné (événement `RoundStarted`).
    Kickoff,
}

fn platform_from_tag(tag: &str) -> Option<Platform> {
    match tag.to_ascii_lowercase().as_str() {
        "epic" | "epicgames" => Some(Platform::Epic),
        "steam" => Some(Platform::Steam),
        "xboxone" | "xbox" | "xbl" => Some(Platform::Xbox),
        "ps4" | "ps5" | "psn" | "playstation" => Some(Platform::Psn),
        _ => None,
    }
}

/// Valide le format de l'identifiant selon la plateforme.
fn valid_id(platform: Platform, id: &str) -> bool {
    match platform {
        Platform::Epic => id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        Platform::Steam => id.len() == 17 && id.starts_with("7656") && id.bytes().all(|b| b.is_ascii_digit()),
        Platform::Xbox => (8..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()),
        Platform::Psn => id.len() >= 4,
    }
}

/// Lit un identifiant typé `Plateforme|id|splitscreen` (ex. `Epic|0123…|0`).
pub fn typed_id(text: &str) -> Option<PlayerId> {
    let mut parts = text.trim().split('|');
    let platform = platform_from_tag(parts.next()?)?;
    let id = parts.next()?;
    parts.next()?.parse::<u32>().ok()?;
    valid_id(platform, id).then(|| PlayerId::new(platform, id))
}

/// Identifiant du joueur local à partir de ce que détecte RL Tracker Suite (`epic` / `steam`
/// et l'identifiant brut du log).
pub fn local_player_id(platform: &str, id: &str) -> Option<PlayerId> {
    let platform = platform_from_tag(platform)?;
    valid_id(platform, id).then(|| PlayerId::new(platform, id))
}

/// Analyse un événement déjà décodé de la Stats API (`UpdateState`, `MatchCreated`, …).
pub fn parse_stats_event(event: &str, data: &Value) -> Vec<ParsedEvent> {
    let mut out = Vec::new();
    if matches!(event, "MatchCreated" | "MatchInitialized" | "MatchDestroyed") {
        out.push(ParsedEvent::LobbyReset);
    }
    if event == "RoundStarted" {
        out.push(ParsedEvent::Kickoff);
    }
    if let Some(players) = data.get("Players").and_then(Value::as_array) {
        for p in players {
            let Some(id) = p.get("PrimaryId").and_then(Value::as_str).and_then(typed_id) else { continue };
            out.push(ParsedEvent::PlayerSeen {
                id,
                name: p.get("Name").and_then(Value::as_str).map(str::to_owned),
                team: p.get("TeamNum").and_then(Value::as_i64).map(Team::from_num),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EPIC: &str = "0123456789abcdef0123456789ABCDEF";

    #[test]
    fn typed_ids_per_platform() {
        let epic = typed_id(&format!("Epic|{EPIC}|0")).unwrap();
        assert_eq!((epic.platform, epic.id.as_str()), (Platform::Epic, EPIC.to_ascii_lowercase().as_str()));
        assert_eq!(typed_id("Steam|76561198012345678|0").unwrap().platform, Platform::Steam);
        assert_eq!(typed_id("XboxOne|2535412345678901|0").unwrap().platform, Platform::Xbox);
        assert_eq!(typed_id("PS4|abc12345def|0").unwrap().platform, Platform::Psn);
    }

    #[test]
    fn rejects_malformed_ids() {
        assert!(typed_id("Epic|tooshort|0").is_none());
        assert!(typed_id("Steam|12345678901234567|0").is_none());
        assert!(typed_id("Xbox|notdigits12345|0").is_none());
        assert!(typed_id(&format!("Epic|{EPIC}")).is_none());
        assert!(typed_id("Unknown|abcd|0").is_none());
    }

    #[test]
    fn local_player_from_detected_platform() {
        assert!(local_player_id("epic", EPIC).is_some());
        assert!(local_player_id("steam", "76561198012345678").is_some());
        assert!(local_player_id("epic", "6897812333f2").is_none());
    }

    #[test]
    fn update_state_lists_players_with_team() {
        let data = json!({"Players":[
            {"Name":"Alpha","PrimaryId":format!("Epic|{EPIC}|0"),"TeamNum":0},
            {"Name":"Beta","PrimaryId":"Steam|76561198012345678|0","TeamNum":1},
            {"Name":"Bot","PrimaryId":"Unknown|0|0","TeamNum":1}]});
        let ev = parse_stats_event("UpdateState", &data);
        assert_eq!(ev.len(), 2);
        match &ev[1] {
            ParsedEvent::PlayerSeen { team, name, .. } => {
                assert_eq!(*team, Some(Team::Orange));
                assert_eq!(name.as_deref(), Some("Beta"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn match_events_reset_and_kickoff() {
        assert_eq!(parse_stats_event("MatchCreated", &json!({})), vec![ParsedEvent::LobbyReset]);
        assert_eq!(parse_stats_event("RoundStarted", &json!({"MatchGuid":"abc"})), vec![ParsedEvent::Kickoff]);
        // Le début du décompte n'est pas encore le coup d'envoi.
        assert!(parse_stats_event("CountdownBegin", &json!({})).is_empty());
        assert!(parse_stats_event("UpdateState", &json!({})).is_empty());
    }
}
