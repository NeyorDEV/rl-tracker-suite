//! Suivi de la session : transforme les événements de la Stats API en stats façon RocketStats.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// IDs de playlist habituels de Rocket League.
pub const PLAYLISTS: [(i64, &str); 12] = [
    (1, "Duel"),
    (2, "Doubles"),
    (3, "Standard"),
    (4, "Chaos"),
    (6, "Partie privée"),
    (10, "Ranked 1v1"),
    (11, "Ranked 2v2"),
    (13, "Ranked 3v3"),
    (27, "Hoops"),
    (28, "Rumble"),
    (29, "Dropshot"),
    (30, "Snow Day"),
];
pub const RANKED: [i64; 7] = [10, 11, 13, 27, 28, 29, 30];
const RECENT_MAX: usize = 20;

pub fn playlist_name(id: Option<i64>) -> String {
    match id {
        None => "Playlist inconnue".to_string(),
        Some(id) => PLAYLISTS
            .iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| format!("Playlist {id}")),
    }
}

pub fn playlist_id_to_mode(id: i64) -> Option<&'static str> {
    match id {
        10 => Some("1v1"),
        11 => Some("2v2"),
        13 => Some("3v3"),
        27 => Some("hoops"),
        28 => Some("rumble"),
        29 => Some("dropshot"),
        30 => Some("snowday"),
        34 => Some("tournaments"),
        _ => None,
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Calcule le rang, la division et le tier numérique (0 à 22) en fonction du MMR et du mode (1v1, 2v2, 3v3).
pub fn calculate_rank(mmr: f64, mode: &str) -> (&'static str, i64, i64) {
    if mmr <= 0.0 {
        return ("Unranked", 1, 0);
    }

    if mode == "1v1" {
        let brackets_1v1: &[(&str, f64, f64, i64)] = &[
            ("Bronze I", 0.0, 155.0, 1),
            ("Bronze II", 155.0, 215.0, 2),
            ("Bronze III", 215.0, 275.0, 3),
            ("Silver I", 275.0, 335.0, 4),
            ("Silver II", 335.0, 395.0, 5),
            ("Silver III", 395.0, 455.0, 6),
            ("Gold I", 455.0, 515.0, 7),
            ("Gold II", 515.0, 575.0, 8),
            ("Gold III", 575.0, 635.0, 9),
            ("Platinum I", 635.0, 695.0, 10),
            ("Platinum II", 695.0, 755.0, 11),
            ("Platinum III", 755.0, 815.0, 12),
            ("Diamond I", 815.0, 875.0, 13),
            ("Diamond II", 875.0, 935.0, 14),
            ("Diamond III", 935.0, 995.0, 15),
            ("Champion I", 995.0, 1055.0, 16),
            ("Champion II", 1055.0, 1115.0, 17),
            ("Champion III", 1115.0, 1175.0, 18),
            ("Grand Champion I", 1175.0, 1235.0, 19),
            ("Grand Champion II", 1235.0, 1295.0, 20),
            ("Grand Champion III", 1295.0, 1355.0, 21),
        ];

        if mmr >= 1355.0 {
            return ("Supersonic Legend", 1, 22);
        }

        for &(name, min, max, tier) in brackets_1v1 {
            if mmr < max {
                let width = (max - min) / 4.0;
                let div = 1 + ((mmr - min) / width).floor().clamp(0.0, 3.0) as i64;
                return (name, div, tier);
            }
        }
        return ("Supersonic Legend", 1, 22);
    }

    // Grille officielle Rocket League pour 2v2 / 3v3 (1656 MMR = GC2 Div 3)
    let brackets: &[(&str, f64, f64, i64)] = &[
        ("Bronze I", 0.0, 175.0, 1),
        ("Bronze II", 175.0, 235.0, 2),
        ("Bronze III", 235.0, 295.0, 3),
        ("Silver I", 295.0, 355.0, 4),
        ("Silver II", 355.0, 415.0, 5),
        ("Silver III", 415.0, 475.0, 6),
        ("Gold I", 475.0, 535.0, 7),
        ("Gold II", 535.0, 595.0, 8),
        ("Gold III", 595.0, 655.0, 9),
        ("Platinum I", 655.0, 715.0, 10),
        ("Platinum II", 715.0, 775.0, 11),
        ("Platinum III", 775.0, 835.0, 12),
        ("Diamond I", 835.0, 915.0, 13),
        ("Diamond II", 915.0, 995.0, 14),
        ("Diamond III", 995.0, 1075.0, 15),
        ("Champion I", 1075.0, 1195.0, 16),
        ("Champion II", 1195.0, 1315.0, 17),
        ("Champion III", 1315.0, 1435.0, 18),
        ("Grand Champion I", 1435.0, 1575.0, 19),
        ("Grand Champion II", 1575.0, 1715.0, 20),
        ("Grand Champion III", 1715.0, 1875.0, 21),
    ];

    if mmr >= 1875.0 {
        return ("Supersonic Legend", 1, 22);
    }

    for &(name, min, max, tier) in brackets {
        if mmr < max {
            let width = (max - min) / 4.0;
            let div = 1 + ((mmr - min) / width).floor().clamp(0.0, 3.0) as i64;
            return (name, div, tier);
        }
    }

    ("Supersonic Legend", 1, 22)
}

// ---------------------------------------------------------------------------
// Données de session (sérialisées en camelCase pour l'interface)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Totals {
    pub goals: i64,
    pub assists: i64,
    pub saves: i64,
    pub shots: i64,
    pub demos: i64,
    pub deaths: i64,
    pub score: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaylistStats {
    pub id: Option<i64>,
    pub name: String,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
    pub streak: i64,
    pub mmr: f64,
    pub session_mmr_change: f64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct RecentMatch {
    pub won: bool,
    pub abandoned: bool,
    pub playlist: String,
    pub at: i64,
    pub goals: i64,
    pub assists: i64,
    pub saves: i64,
    pub demos: i64,
    pub mmr_change: f64,
    pub mmr_after: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub started_at: i64,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
    pub streak: i64,
    pub best_streak: i64,
    pub worst_streak: i64,
    pub totals: Totals,
    pub playlists: BTreeMap<String, PlaylistStats>,
    pub recent: Vec<RecentMatch>,
    pub current_mmr: f64,
    pub session_mmr_change: f64,
    pub last_match_mmr_change: f64,
    pub rank_name: String,
    pub division: i64,
    pub rank_tier: i64,
    pub focused_playlist: String,
    pub playlists_mmr: BTreeMap<String, f64>,
}

impl Default for Session {
    fn default() -> Self {
        let (rank_name, division, rank_tier) = calculate_rank(855.0, "2v2");
        Session {
            started_at: now_ms(),
            games: 0,
            wins: 0,
            losses: 0,
            streak: 0,
            best_streak: 0,
            worst_streak: 0,
            totals: Totals::default(),
            playlists: BTreeMap::new(),
            recent: Vec::new(),
            current_mmr: 855.0,
            session_mmr_change: 0.0,
            last_match_mmr_change: 0.0,
            rank_name: rank_name.to_string(),
            division,
            rank_tier,
            focused_playlist: "2v2".into(),
            playlists_mmr: BTreeMap::new(),
        }
    }
}

fn next_streak(current: i64, won: bool) -> i64 {
    if won { current.max(0) + 1 } else { current.min(0) - 1 }
}

// ---------------------------------------------------------------------------
// Règles de comptage et effets renvoyés à l'appelant
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CountRules {
    pub online_only: bool,
    /// "all" | "ranked" | "custom"
    pub mode: String,
    pub playlists: Vec<i64>,
    pub count_leave_as_loss: bool,
    pub mmr_delta: f64,
    pub focused_playlist: String,
    pub auto_switch_playlist: bool,
}

impl Default for CountRules {
    fn default() -> Self {
        CountRules {
            online_only: true,
            mode: "all".into(),
            playlists: vec![],
            count_leave_as_loss: true,
            mmr_delta: 9.0,
            focused_playlist: "2v2".into(),
            auto_switch_playlist: true,
        }
    }
}

impl CountRules {
    fn should_count(&self, m: &MatchState) -> bool {
        if self.online_only && m.guid.is_none() {
            return false;
        }
        match self.mode.as_str() {
            "ranked" => m.playlist_id.map_or(false, |id| RANKED.contains(&id)),
            "custom" => m.playlist_id.map_or(false, |id| self.playlists.contains(&id)),
            _ => true,
        }
    }
}

/// Ce qui a changé après un événement (l'appelant sauvegarde / rafraîchit l'affichage).
#[derive(Default, Debug)]
pub struct Effects {
    pub session_changed: bool,
    /// Un match vient de se terminer et d'être compté (seul moment où le MMR bouge).
    pub match_finished: bool,
    pub status_changed: bool,
    /// Joueur reconnu via la caméra : (PrimaryId, nom)
    pub detected: Option<(String, String)>,
    pub logs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Suivi du match en cours
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Me {
    id: Option<String>,
    name: String,
}

#[derive(Clone, Debug)]
struct MatchState {
    guid: Option<String>,
    playlist_id: Option<i64>,
    me: Option<Me>,
    my_team: Option<i64>,
    stats: Value,
    deaths: i64,
    winner_team: Option<i64>,
    started: bool,
    done: bool,
}

impl MatchState {
    fn new(guid: Option<String>) -> Self {
        MatchState {
            guid,
            playlist_id: None,
            me: None,
            my_team: None,
            stats: Value::Null,
            deaths: 0,
            winner_team: None,
            started: false,
            done: false,
        }
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
fn int_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}
fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn guid_of(d: &Value) -> Option<String> {
    str_of(d, "MatchGuid").filter(|s| !s.is_empty()).map(String::from)
}

pub struct Tracker {
    pub session: Session,
    current: Option<MatchState>,
}

impl Tracker {
    pub fn new(session: Session) -> Self {
        Tracker { session, current: None }
    }

    pub fn in_match(&self) -> bool {
        self.current.as_ref().map_or(false, |m| m.started && !m.done)
    }

    pub fn reset(&mut self) {
        let current_mmr = self.session.current_mmr;
        let focused = self.session.focused_playlist.clone();
        let mut s = Session::default();
        s.focused_playlist = focused.clone();
        if current_mmr > 0.0 {
            s.current_mmr = current_mmr;
            let (name, div, tier) = calculate_rank(current_mmr, &focused);
            s.rank_name = name.to_string();
            s.division = div;
            s.rank_tier = tier;
        }
        self.session = s;
    }

    pub fn set_mmr(&mut self, mmr: f64, focused_playlist: &str) {
        self.session.current_mmr = mmr.max(0.0);
        self.session.focused_playlist = focused_playlist.to_string();
        let (name, div, tier) = calculate_rank(self.session.current_mmr, focused_playlist);
        self.session.rank_name = name.to_string();
        self.session.division = div;
        self.session.rank_tier = tier;
    }

    pub fn update_focus(&mut self, focused_playlist: &str) {
        self.session.focused_playlist = focused_playlist.to_string();
        if let Some(&saved) = self.session.playlists_mmr.get(focused_playlist) {
            self.session.current_mmr = saved;
        }
        let (name, div, tier) = calculate_rank(self.session.current_mmr, focused_playlist);
        self.session.rank_name = name.to_string();
        self.session.division = div;
        self.session.rank_tier = tier;
    }

    pub fn switch_playlist(&mut self, new_mode: &str, e: &mut Effects) {
        if self.session.focused_playlist == new_mode {
            return;
        }
        self.session.playlists_mmr.insert(self.session.focused_playlist.clone(), self.session.current_mmr);
        self.session.focused_playlist = new_mode.to_string();
        if let Some(&saved_mmr) = self.session.playlists_mmr.get(new_mode) {
            self.session.current_mmr = saved_mmr;
        }
        let (name, div, tier) = calculate_rank(self.session.current_mmr, new_mode);
        self.session.rank_name = name.to_string();
        self.session.division = div;
        self.session.rank_tier = tier;
        e.session_changed = true;
        e.status_changed = true;
        e.logs.push(format!("Auto-switch mode : passage en {new_mode}"));
    }

    pub fn handle(&mut self, event: &str, d: &Value, identity: Option<&str>, rules: &CountRules) -> Effects {
        let mut e = Effects::default();
        match event {
            "MatchCreated" | "MatchInitialized" => {
                self.ensure_match(d);
            }
            "RoundStarted" => {
                let m = self.ensure_match(d);
                if !m.started {
                    m.started = true;
                    e.status_changed = true;
                }
            }
            "UpdateState" => self.on_tick(d, identity, rules, &mut e),
            "StatfeedEvent" => self.on_stat(d),
            "MatchEnded" => {
                let winner = d.get("WinnerTeamNum").and_then(Value::as_i64);
                self.finish(winner, rules, &mut e);
            }
            "MatchDestroyed" => self.leave(rules, &mut e),
            _ => {}
        }
        e
    }

    /// Le socket s'est fermé : si un match était en cours, on le traite comme une sortie.
    pub fn connection_lost(&mut self, rules: &CountRules) -> Effects {
        let mut e = Effects::default();
        self.leave(rules, &mut e);
        e
    }

    fn ensure_match(&mut self, d: &Value) -> &mut MatchState {
        let guid = guid_of(d);
        let reuse = match (&self.current, &guid) {
            (Some(m), Some(g)) => m.guid.as_deref() == Some(g.as_str()),
            (Some(m), None) => !m.done,
            (None, _) => false,
        };
        if !reuse {
            self.current = Some(MatchState::new(guid));
        }
        self.current.as_mut().expect("match présent")
    }

    fn on_tick(&mut self, d: &Value, identity: Option<&str>, rules: &CountRules, e: &mut Effects) {
        let playlist_id = d.get("Game").and_then(|g| g.get("PlaylistId")).and_then(Value::as_i64);
        if rules.auto_switch_playlist {
            if let Some(id) = playlist_id {
                if let Some(mode) = playlist_id_to_mode(id) {
                    if self.session.focused_playlist != mode {
                        self.switch_playlist(mode, e);
                    }
                }
            }
        }

        let m = self.ensure_match(d);
        let null = Value::Null;
        let g = d.get("Game").unwrap_or(&null);
        let no_players = Vec::new();
        let players = d.get("Players").and_then(Value::as_array).unwrap_or(&no_players);

        if let Some(id) = playlist_id {
            m.playlist_id = Some(id);
        }
        if bool_of(g, "bHasWinner") {
            if let Some(winner) = str_of(g, "Winner").filter(|s| !s.is_empty()) {
                let team = g
                    .get("Teams")
                    .and_then(Value::as_array)
                    .and_then(|teams| teams.iter().find(|t| str_of(t, "Name") == Some(winner)));
                if let Some(t) = team {
                    m.winner_team = t.get("TeamNum").and_then(Value::as_i64);
                }
            }
        }

        if m.me.is_none() {
            m.me = identify(players, g, identity, e);
        }
        let Some(me) = m.me.clone() else { return };

        let found = players.iter().find(|p| match &me.id {
            Some(id) => str_of(p, "PrimaryId") == Some(id.as_str()),
            None => str_of(p, "Name") == Some(me.name.as_str()),
        });
        if let Some(p) = found {
            m.my_team = p.get("TeamNum").and_then(Value::as_i64);
            m.stats = p.clone();
            if let Some(name) = str_of(p, "Name") {
                m.me = Some(Me { id: me.id, name: name.to_string() });
            }
        }
    }

    fn on_stat(&mut self, d: &Value) {
        let Some(m) = self.current.as_mut() else { return };
        let Some(me) = &m.me else { return };
        let victim = d.get("SecondaryTarget").and_then(|t| str_of(t, "Name"));
        if str_of(d, "EventName") == Some("Demolish") && victim == Some(me.name.as_str()) {
            m.deaths += 1;
        }
    }

    fn finish(&mut self, winner: Option<i64>, rules: &CountRules, e: &mut Effects) {
        let snapshot = match self.current.as_mut() {
            Some(m) if !m.done => {
                m.done = true;
                m.clone()
            }
            _ => return,
        };
        match snapshot.my_team {
            None => e.logs.push("Fin de match ignorée : joueur non identifié".into()),
            Some(team) => self.record(&snapshot, winner == Some(team), false, rules, e),
        }
        e.status_changed = true;
    }

    fn leave(&mut self, rules: &CountRules, e: &mut Effects) {
        e.status_changed = true;
        let Some(mut m) = self.current.take() else { return };
        if m.done || !m.started || m.my_team.is_none() {
            return;
        }
        m.done = true;
        if let Some(winner) = m.winner_team {
            // Le jeu avait déjà désigné un gagnant (forfait, fin sans MatchEnded).
            let won = Some(winner) == m.my_team;
            self.record(&m, won, false, rules, e);
        } else if rules.count_leave_as_loss {
            self.record(&m, false, true, rules, e);
        }
    }

    fn record(&mut self, m: &MatchState, won: bool, abandoned: bool, rules: &CountRules, e: &mut Effects) {
        let name = playlist_name(m.playlist_id);
        if !rules.should_count(m) {
            e.logs.push(format!("Partie non comptée ({name})"));
            return;
        }
        let s = &mut self.session;
        s.games += 1;
        if won { s.wins += 1 } else { s.losses += 1 }
        s.streak = next_streak(s.streak, won);
        s.best_streak = s.best_streak.max(s.streak);
        s.worst_streak = s.worst_streak.min(s.streak);

        let st = &m.stats;
        let t = &mut s.totals;
        t.goals += int_of(st, "Goals");
        t.assists += int_of(st, "Assists");
        t.saves += int_of(st, "Saves");
        t.shots += int_of(st, "Shots");
        t.demos += int_of(st, "Demos");
        t.score += int_of(st, "Score");
        t.deaths += m.deaths;

        let delta = if won { rules.mmr_delta } else { -rules.mmr_delta };
        s.current_mmr = (s.current_mmr + delta).max(0.0);
        s.playlists_mmr.insert(s.focused_playlist.clone(), s.current_mmr);
        s.session_mmr_change += delta;
        s.last_match_mmr_change = delta;
        let (r_name, r_div, r_tier) = calculate_rank(s.current_mmr, &s.focused_playlist);
        s.rank_name = r_name.to_string();
        s.division = r_div;
        s.rank_tier = r_tier;

        let key = m.playlist_id.map_or_else(|| "unknown".to_string(), |id| id.to_string());
        let pl = s.playlists.entry(key).or_insert_with(|| PlaylistStats {
            id: m.playlist_id,
            name: name.clone(),
            ..Default::default()
        });
        pl.games += 1;
        if won { pl.wins += 1 } else { pl.losses += 1 }
        pl.streak = next_streak(pl.streak, won);
        pl.mmr = s.current_mmr;
        pl.session_mmr_change += delta;

        s.recent.push(RecentMatch {
            won,
            abandoned,
            playlist: name.clone(),
            at: now_ms(),
            goals: int_of(st, "Goals"),
            assists: int_of(st, "Assists"),
            saves: int_of(st, "Saves"),
            demos: int_of(st, "Demos"),
            mmr_change: delta,
            mmr_after: s.current_mmr,
        });
        if s.recent.len() > RECENT_MAX {
            let extra = s.recent.len() - RECENT_MAX;
            s.recent.drain(..extra);
        }

        e.logs.push(format!(
            "{}{} - {} - streak {} - MMR: {:.1} ({:+.1})",
            if won { "Victoire" } else { "Défaite" },
            if abandoned { " (abandon)" } else { "" },
            name,
            s.streak,
            s.current_mmr,
            delta
        ));
        e.session_changed = true;
        e.match_finished = true;
    }
}

/// Retrouve le joueur local : par identifiant connu, sinon via la caméra
/// (quand tu joues, la caméra suit ta propre voiture).
fn identify(players: &[Value], g: &Value, identity: Option<&str>, e: &mut Effects) -> Option<Me> {
    let mut found = None;
    let mut via_camera = false;

    if let Some(id_str) = identity.filter(|s| !s.is_empty()) {
        if let Some(rest) = id_str.strip_prefix("NAME:") {
            let (plat, pseudo) = rest.split_once('|').unwrap_or(("", rest));
            found = players.iter().find(|p| {
                let p_name = str_of(p, "Name").unwrap_or("");
                let p_id = str_of(p, "PrimaryId").unwrap_or("");
                let name_match = p_name.eq_ignore_ascii_case(pseudo);
                let plat_match = plat.is_empty() || p_id.to_lowercase().starts_with(&plat.to_lowercase());
                name_match && plat_match
            });
        } else {
            found = players.iter().find(|p| str_of(p, "PrimaryId") == Some(id_str));
        }
    }

    if found.is_none() && bool_of(g, "bHasTarget") && !bool_of(g, "bReplay") {
        if let Some(target) = g.get("Target").and_then(|t| str_of(t, "Name")) {
            found = players.iter().find(|p| str_of(p, "Name") == Some(target));
            via_camera = found.is_some();
        }
    }
    let p = found?;
    let name = str_of(p, "Name").unwrap_or_default().to_string();
    let id = str_of(p, "PrimaryId").filter(|s| !s.is_empty()).map(String::from);
    if via_camera {
        if let Some(id) = &id {
            e.detected = Some((id.clone(), name.clone()));
        }
    }
    e.logs.push(format!("Joueur suivi : {name}"));
    Some(Me { id, name })
}
