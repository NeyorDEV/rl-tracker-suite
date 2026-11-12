//! Historique des rencontres : contre qui et avec qui on a déjà joué, et avec quel résultat.
//!
//! Alimenté uniquement par la Stats API (identifiants des joueurs de chaque match terminé),
//! sans aucun appel réseau. Les joueurs sont repérés par leur clé `plateforme:id`.

use crate::view::EncounterView;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Nombre maximum de joueurs gardés ; au-delà, les plus anciens sont oubliés.
const MAX_PLAYERS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tally {
    pub wins: u32,
    pub losses: u32,
}

impl Tally {
    fn add(&mut self, won: bool) {
        if won {
            self.wins += 1;
        } else {
            self.losses += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Encounter {
    pub name: String,
    /// Mes résultats quand il était dans l'équipe adverse.
    pub against: Tally,
    /// Mes résultats quand il était dans mon équipe.
    pub with: Tally,
    /// Heure Unix du dernier match partagé.
    pub last_seen: i64,
}

/// Un joueur présent dans un match terminé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub key: String,
    pub name: String,
    pub team: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncounterBook {
    pub players: HashMap<String, Encounter>,
}

impl EncounterBook {
    /// Enregistre un match terminé. `me_key` est exclu ; les joueurs sans équipe connue aussi.
    pub fn record_match(&mut self, me_key: Option<&str>, my_team: i64, won: bool, players: &[Participant], now: i64) {
        for p in players {
            if Some(p.key.as_str()) == me_key || p.team < 0 {
                continue;
            }
            let entry = self.players.entry(p.key.clone()).or_default();
            if !p.name.is_empty() {
                entry.name = p.name.clone();
            }
            if p.team == my_team {
                entry.with.add(won);
            } else {
                entry.against.add(won);
            }
            entry.last_seen = now;
        }
        self.prune();
    }

    fn prune(&mut self) {
        if self.players.len() <= MAX_PLAYERS {
            return;
        }
        let mut by_age: Vec<(i64, String)> = self.players.iter().map(|(k, e)| (e.last_seen, k.clone())).collect();
        by_age.sort();
        for (_, key) in by_age.into_iter().take(self.players.len() - MAX_PLAYERS) {
            self.players.remove(&key);
        }
    }

    /// Vues à joindre au lobby, par clé de joueur.
    pub fn views(&self) -> HashMap<String, EncounterView> {
        self.players
            .iter()
            .map(|(key, e)| {
                (
                    key.clone(),
                    EncounterView {
                        against_wins: e.against.wins,
                        against_losses: e.against.losses,
                        with_wins: e.with.wins,
                        with_losses: e.with.losses,
                        last_seen: e.last_seen,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(key: &str, team: i64) -> Participant {
        Participant { key: key.into(), name: key.to_uppercase(), team }
    }

    #[test]
    fn counts_results_against_and_with_each_player() {
        let mut book = EncounterBook::default();
        let lobby = [p("epic:me", 0), p("epic:mate", 0), p("epic:foe", 1)];
        book.record_match(Some("epic:me"), 0, true, &lobby, 100);
        book.record_match(Some("epic:me"), 0, false, &lobby, 200);
        book.record_match(Some("epic:me"), 0, true, &lobby, 300);

        assert!(!book.players.contains_key("epic:me"), "on ne se compte pas soi-même");
        let foe = &book.players["epic:foe"];
        assert_eq!((foe.against.wins, foe.against.losses), (2, 1));
        assert_eq!((foe.with.wins, foe.with.losses), (0, 0));
        assert_eq!(foe.last_seen, 300);
        let mate = &book.players["epic:mate"];
        assert_eq!((mate.with.wins, mate.with.losses), (2, 1));
    }

    #[test]
    fn same_player_can_be_foe_then_teammate() {
        let mut book = EncounterBook::default();
        book.record_match(Some("epic:me"), 0, true, &[p("epic:x", 1)], 1);
        book.record_match(Some("epic:me"), 1, false, &[p("epic:x", 1)], 2);
        let x = &book.players["epic:x"];
        assert_eq!((x.against.wins, x.with.losses), (1, 1));
        let view = &book.views()["epic:x"];
        assert_eq!((view.against_wins, view.with_losses, view.last_seen), (1, 1, 2));
    }

    #[test]
    fn keeps_only_the_most_recent_players() {
        let mut book = EncounterBook::default();
        for i in 0..(MAX_PLAYERS as i64 + 10) {
            book.record_match(None, 0, true, &[p(&format!("epic:{i}"), 1)], i);
        }
        assert_eq!(book.players.len(), MAX_PLAYERS);
        assert!(!book.players.contains_key("epic:0"));
        assert!(book.players.contains_key(&format!("epic:{}", MAX_PLAYERS as i64 + 9)));
    }
}
