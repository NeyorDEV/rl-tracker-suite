//! Orchestration : Stats API (via RL Tracker Suite) → parseur → cache / file d'attente →
//! heuristiques → snapshots du lobby.
//!
//! Le moteur ne se connecte à rien lui-même : l'application lui transmet les événements de la
//! Stats API, le joueur local qu'elle a détecté et la WebView qui interroge Tracker Network.

mod demo;
pub mod lobby;

use lobby::{Lobby, Upsert};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracker_cache::Cache;
use tracker_core::{unix_now, Config, LobbySnapshot, Platform, PlayerId, Relation, Team};
use tracker_heuristics::EntryState;
use tracker_parser::{parse_stats_event, ParsedEvent};
use tracker_scraper::{
    FetchOptions, Fetcher, HybridFetcher, Priority, ScrapeError, SourceFetcher, TrackerQueue, TrnApiSource,
    WebFetch, WebSource,
};

/// Un joueur dont la recherche vient d'échouer n'est pas redemandé pendant ce délai.
const FAILURE_MEMORY_SECS: i64 = 600;
/// Délai minimum entre deux recherches de profil.
const LOOKUP_INTERVAL: Duration = Duration::from_millis(3000);

/// Pseudo masqué par le mode anonyme du jeu (« Player 3 ») : inutile de le chercher.
fn is_anonymous_name(name: &str) -> bool {
    name.trim()
        .to_ascii_lowercase()
        .strip_prefix("player ")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Cache(#[from] tracker_cache::CacheError),
    #[error(transparent)]
    Scrape(#[from] ScrapeError),
}

/// Désactive la source WebView quand l'utilisateur la coupe dans les paramètres.
struct Gated {
    enabled: Arc<dyn Fn() -> bool + Send + Sync>,
    inner: Arc<dyn Fetcher>,
}

#[async_trait::async_trait]
impl Fetcher for Gated {
    async fn fetch(
        &self,
        id: &PlayerId,
        opts: FetchOptions,
    ) -> Result<tracker_core::PlayerProfile, ScrapeError> {
        if (self.enabled)() {
            self.inner.fetch(id, opts).await
        } else {
            Err(ScrapeError::NoSource)
        }
    }
}

struct Inner {
    cfg: RwLock<Config>,
    cache: Arc<Cache>,
    queue: Arc<TrackerQueue>,
    lobby: Mutex<Lobby>,
    snapshots: broadcast::Sender<LobbySnapshot>,
    /// Un message par coup d'envoi (fin du décompte), pour masquer l'overlay.
    kickoffs: broadcast::Sender<()>,
    /// Compte local détecté par l'application à chaque lancement du jeu.
    detected_self: Mutex<Option<(PlayerId, Option<String>)>>,
    /// Affiche un lobby fictif à la place du vrai (bouton « Prévisualiser »).
    preview: AtomicBool,
    /// Joueurs dont la recherche a échoué récemment (heure Unix), pour ne pas insister.
    failures: Mutex<HashMap<PlayerId, i64>>,
}

pub struct Engine {
    inner: Arc<Inner>,
    fetcher: Arc<dyn Fetcher>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(
        cfg: Config,
        cache_path: &Path,
        web: Option<Arc<dyn WebFetch>>,
    ) -> Result<Arc<Self>, EngineError> {
        let cache = Arc::new(Cache::open(cache_path)?);
        let ttl = cfg.cache_ttl_hours as i64 * 3600;
        if let Err(e) = cache.purge_expired(ttl * 7) {
            tracing::warn!("purge du cache impossible : {e}");
        }
        let (snapshots, _) = broadcast::channel(32);
        let (kickoffs, _) = broadcast::channel(8);
        let inner = Arc::new(Inner {
            cfg: RwLock::new(cfg),
            cache,
            queue: TrackerQueue::new(),
            lobby: Mutex::new(Lobby::default()),
            snapshots,
            kickoffs,
            detected_self: Mutex::new(None),
            preview: AtomicBool::new(false),
            failures: Mutex::default(),
        });

        let mut sources: Vec<Arc<dyn Fetcher>> = Vec::new();
        if let Some(w) = web {
            let i = inner.clone();
            sources.push(Arc::new(Gated {
                enabled: Arc::new(move || i.cfg.read().map(|c| c.use_webview_scraper).unwrap_or(true)),
                inner: Arc::new(SourceFetcher::new(WebSource::new(w))),
            }));
        }
        let i = inner.clone();
        let key = Arc::new(move || i.cfg.read().ok().and_then(|c| c.trn_api_key.clone()));
        sources.push(Arc::new(SourceFetcher::new(TrnApiSource::new(key)?)));

        Ok(Arc::new(Self {
            inner,
            fetcher: Arc::new(HybridFetcher::new(sources)),
            tasks: Mutex::default(),
        }))
    }

    /// Flux de snapshots pour l'overlay (événements Tauri) et la source OBS.
    pub fn subscribe(&self) -> broadcast::Receiver<LobbySnapshot> {
        self.inner.snapshots.subscribe()
    }

    /// Notifie chaque coup d'envoi détecté par la Stats API.
    pub fn subscribe_kickoff(&self) -> broadcast::Receiver<()> {
        self.inner.kickoffs.subscribe()
    }

    pub fn snapshot(&self) -> LobbySnapshot {
        self.inner.build_snapshot()
    }

    pub fn config(&self) -> Config {
        self.inner.cfg.read().map(|c| c.clone()).unwrap_or_default()
    }

    /// Active ou coupe la prévisualisation de l'overlay avec un lobby fictif.
    pub fn set_preview(&self, enabled: bool) {
        self.inner.preview.store(enabled, Ordering::SeqCst);
        self.inner.emit();
    }

    pub fn is_preview(&self) -> bool {
        self.inner.preview.load(Ordering::SeqCst)
    }

    /// Applique une nouvelle configuration, à chaud.
    pub fn update_config(&self, cfg: Config) {
        if let Ok(mut c) = self.inner.cfg.write() {
            *c = cfg;
        }
        self.inner.emit();
    }

    /// Démarre la file d'attente et le worker réseau. À appeler depuis un runtime tokio.
    pub async fn start(self: &Arc<Self>) {
        let inner = self.inner.clone();
        let cfg = self.config();
        let mut tasks = Vec::new();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let i = inner.clone();
        let options: Arc<dyn Fn() -> FetchOptions + Send + Sync> = Arc::new(move || FetchOptions {
            with_sessions: i
                .cfg
                .read()
                .map(|c| c.filters.on_tilt || c.filters.smurf_suspect)
                .unwrap_or(false),
            lookup: None,
        });
        tasks.push(inner.queue.spawn_worker(
            self.fetcher.clone(),
            inner.cache.clone(),
            cfg.cache_ttl_hours as i64 * 3600,
            options,
            LOOKUP_INTERVAL,
            out_tx,
        ));
        let i = inner.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(outcome) = out_rx.recv().await {
                i.apply_outcome(outcome);
            }
        }));

        *self.tasks.lock().unwrap() = tasks;
        inner.emit();
    }

    /// Événement de la Stats API déjà décodé (`UpdateState`, `RoundStarted`, …).
    pub fn ingest_stats(&self, event: &str, data: &Value) {
        for e in parse_stats_event(event, data) {
            self.inner.handle_parsed(e);
        }
    }

    /// Rocket League vient d'être (re)lancé : le compte a pu changer (Epic <-> Steam), on oublie
    /// l'ancien et le lobby en cours jusqu'à la prochaine détection.
    pub fn game_launched(&self) {
        *self.inner.detected_self.lock().unwrap() = None;
        self.inner.queue.clear();
        self.inner.lobby.lock().unwrap().reset();
        self.inner.emit();
    }

    /// Compte local détecté par RL Tracker Suite (`platform` = `epic` ou `steam`).
    /// Renvoie `false` si l'identifiant n'a pas le format attendu.
    pub fn set_local_player(&self, platform: &str, id: &str, name: &str) -> bool {
        let Some(pid) = tracker_parser::local_player_id(platform, id) else { return false };
        tracing::info!("joueur local : {name} ({pid})");
        self.inner.lobby.lock().unwrap().remove(&pid);
        *self.inner.detected_self.lock().unwrap() = Some((pid, Some(name.to_string()).filter(|n| !n.is_empty())));
        self.inner.emit();
        true
    }

    /// Injecte un identifiant à la main (utile pour tester sans lancer le jeu).
    pub fn inject_player(&self, id: PlayerId, name: Option<String>, team: Option<Team>) {
        self.inner.handle_parsed(ParsedEvent::PlayerSeen { id, name, team });
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(tasks) = self.tasks.lock() {
            for t in tasks.iter() {
                t.abort();
            }
        }
    }
}

impl Inner {
    fn handle_parsed(&self, ev: ParsedEvent) {
        match ev {
            ParsedEvent::Kickoff => {
                let _ = self.kickoffs.send(());
            }
            ParsedEvent::LobbyReset => {
                self.queue.clear();
                self.lobby.lock().unwrap().reset();
                self.emit();
            }
            ParsedEvent::PlayerSeen { id, name, team } => self.handle_seen(id, name, team),
        }
    }

    fn is_self(&self, id: &PlayerId, name: Option<&str>) -> bool {
        let detected = self.detected_self.lock().unwrap();
        let Some((did, dname)) = detected.as_ref() else { return false };
        let by_name = matches!((dname.as_deref(), name), (Some(me), Some(n)) if me.eq_ignore_ascii_case(n));
        did == id || by_name
    }

    fn handle_seen(&self, id: PlayerId, name: Option<String>, team: Option<Team>) {
        if self.is_self(&id, name.as_deref()) {
            // On ne se profile pas soi-même, mais on en déduit notre équipe.
            let mut lobby = self.lobby.lock().unwrap();
            if team.is_some_and(|t| lobby.set_self_team(t)) {
                lobby.recompute_relations();
                drop(lobby);
                self.emit();
            }
            return;
        }

        let lookup = name.clone().filter(|n| !n.is_empty());
        let (state, relation, loading) = {
            let mut lobby = self.lobby.lock().unwrap();
            let state = lobby.upsert(&id, name, team);
            let loading = lobby.entries().iter().any(|e| e.id == id && matches!(e.state, EntryState::Loading));
            (state, lobby.relation_for(&id), loading)
        };
        if state == Upsert::Unchanged {
            return;
        }
        // Hors Steam, Tracker Network cherche par pseudo : on attend de le connaître.
        let needs_name = id.platform != Platform::Steam;
        if state == Upsert::Changed && loading && lookup.is_some() && needs_name {
            self.request_profile(id, relation, lookup);
            self.emit();
            return;
        }

        if state == Upsert::New {
            let ttl = self.cfg.read().unwrap().cache_ttl_hours as i64 * 3600;
            match self.cache.get_fresh(&id, ttl) {
                // Chemin rapide : affichage immédiat depuis SQLite.
                Ok(Some(profile)) => {
                    self.lobby.lock().unwrap().set_profile(profile);
                }
                Ok(None) => self.request_profile(id, relation, lookup),
                Err(e) => {
                    tracing::warn!("lecture du cache impossible : {e}");
                    self.request_profile(id, relation, lookup);
                }
            }
        }
        self.emit();
    }

    /// Demande le profil d'un joueur à Tracker Network, sauf si c'est inutile ou déjà perdu
    /// d'avance : pseudo anonyme (« Player 3 »), pseudo encore inconnu, échec très récent.
    /// Chaque requête évitée ménage le site (qui bloque les clients trop bavards).
    fn request_profile(&self, id: PlayerId, relation: Relation, lookup: Option<String>) {
        let needs_name = id.platform != Platform::Steam;
        if needs_name && lookup.is_none() {
            return;
        }
        if needs_name && lookup.as_deref().is_some_and(is_anonymous_name) {
            self.lobby.lock().unwrap().set_failed(&id);
            return;
        }
        let recently_failed = self
            .failures
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|at| unix_now() - at < FAILURE_MEMORY_SECS);
        if recently_failed {
            self.lobby.lock().unwrap().set_failed(&id);
            return;
        }
        self.queue.enqueue_named(id, priority_of(relation), lookup);
    }

    fn apply_outcome(&self, outcome: tracker_scraper::FetchOutcome) {
        {
            let mut lobby = self.lobby.lock().unwrap();
            match outcome {
                tracker_scraper::FetchOutcome::Fetched(p) => {
                    lobby.set_profile(*p);
                }
                tracker_scraper::FetchOutcome::Failed { id, error } => {
                    tracing::info!("profil {id} indisponible : {error}");
                    // Un blocage général (429) n'est pas la faute du joueur : on le réessaiera.
                    if !error.contains("429") {
                        self.failures.lock().unwrap().insert(id.clone(), unix_now());
                    }
                    lobby.set_failed(&id);
                }
            }
        }
        self.emit();
    }

    fn build_snapshot(&self) -> LobbySnapshot {
        let cfg = self.cfg.read().unwrap().clone();
        if self.preview.load(Ordering::SeqCst) {
            let entries = demo::demo_entries();
            return LobbySnapshot {
                pending: 1,
                players: tracker_heuristics::build_views(&entries, &cfg),
                updated_at: unix_now(),
                me: self.me_label(),
            };
        }
        let lobby = self.lobby.lock().unwrap();
        LobbySnapshot {
            players: tracker_heuristics::build_views(lobby.entries(), &cfg),
            pending: lobby.loading_count(),
            updated_at: unix_now(),
            me: self.me_label(),
        }
    }

    fn me_label(&self) -> Option<String> {
        let d = self.detected_self.lock().unwrap();
        d.as_ref().map(|(id, name)| {
            format!("{} · {}", name.as_deref().unwrap_or("?"), id.platform.as_str())
        })
    }

    fn emit(&self) {
        let _ = self.snapshots.send(self.build_snapshot());
    }
}

fn priority_of(r: Relation) -> Priority {
    match r {
        Relation::Opponent => Priority::Opponent,
        Relation::Unknown => Priority::Unknown,
        Relation::Teammate => Priority::Teammate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tracker_core::{PlayerProfile, PlayerStatus, PlaylistKind, PlaylistStat};

    const EPIC: &str = "6897812333f244858dddff1fbb06554f";

    fn id(n: u32) -> PlayerId {
        PlayerId::new(Platform::Steam, format!("7656119800000{n:04}"))
    }

    fn engine(cfg: Config) -> Arc<Engine> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("lt-engine-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Engine::new(cfg, &dir.join("cache.db"), None).unwrap()
    }

    fn smurf_profile(n: u32) -> PlayerProfile {
        PlayerProfile {
            id: id(n),
            display_name: "Smurfy".into(),
            total_wins: 90,
            peak_tier: None,
            playlists: vec![PlaylistStat { kind: PlaylistKind::Standard, tier: 19, mmr: None, matches: 50 }],
            saves_per_match: None,
            goals_per_match: None,
            shots_per_match: None,
            loss_streak: 0,
            recent: None,
            fetched_at: unix_now(),
        }
    }

    #[test]
    fn cached_player_is_ready_immediately_without_network() {
        let e = engine(Config::default());
        e.inner.cache.put(&smurf_profile(1)).unwrap();
        e.inject_player(id(1), Some("raw".into()), Some(Team::Orange));
        let snap = e.snapshot();
        assert_eq!(snap.players.len(), 1);
        assert_eq!(snap.players[0].status, PlayerStatus::Ready);
        assert!(snap.players[0].badge_labels.iter().any(|l| l == "Potentiel smurf"));
        assert_eq!(snap.pending, 0);
    }

    #[test]
    fn uncached_player_is_loading_and_queued_by_priority() {
        let e = engine(Config::default());
        assert!(e.set_local_player("epic", EPIC, "Me"));
        let me = PlayerId::new(Platform::Epic, EPIC);
        e.inject_player(me, Some("Me".into()), Some(Team::Blue));
        e.inject_player(id(1), Some("Mate".into()), Some(Team::Blue));
        e.inject_player(id(2), Some("Foe".into()), Some(Team::Orange));
        let snap = e.snapshot();
        assert_eq!(snap.players.len(), 2, "soi-même n'est pas profilé");
        assert_eq!(snap.players[0].name, "Foe", "adversaires en premier");
        assert_eq!(snap.pending, 2);
        // L'adversaire est servi avant le coéquipier même s'il est arrivé après.
        assert_eq!(e.inner.queue.pop_next(), Some(id(2)));
    }

    #[test]
    fn anonymous_names_are_never_looked_up() {
        assert!(is_anonymous_name("Player 3") && is_anonymous_name(" player 12 "));
        assert!(!is_anonymous_name("Player") && !is_anonymous_name("Player One") && !is_anonymous_name("Neyor"));

        let e = engine(Config::default());
        let epic = PlayerId::new(Platform::Epic, EPIC);
        e.inject_player(epic.clone(), Some("Player 2".into()), Some(Team::Orange));
        let snap = e.snapshot();
        assert_eq!(snap.players[0].status, PlayerStatus::Failed);
        assert_eq!(e.inner.queue.pending(), 0, "aucune requête pour un pseudo anonyme");
        // Un compte Steam se cherche par identifiant : le pseudo anonyme n'y change rien.
        e.inject_player(id(5), Some("Player 4".into()), Some(Team::Orange));
        assert_eq!(e.inner.queue.pending(), 1);
    }

    #[test]
    fn a_recent_failure_is_not_retried_but_a_global_block_is() {
        let e = engine(Config::default());
        e.inner.apply_outcome(tracker_scraper::FetchOutcome::Failed { id: id(1), error: "profil introuvable".into() });
        e.inner.apply_outcome(tracker_scraper::FetchOutcome::Failed { id: id(2), error: "HTTP 429".into() });
        e.inner.lobby.lock().unwrap().reset();
        e.inject_player(id(1), Some("Gone".into()), None);
        e.inject_player(id(2), Some("Busy".into()), None);
        assert_eq!(e.inner.queue.pending(), 1, "seul le joueur bloqué par le 429 est redemandé");
        assert_eq!(e.inner.queue.pop_next(), Some(id(2)));
    }

    #[test]
    fn only_opponents_filter_applies_live() {
        let mut cfg = Config::default();
        let e = engine(cfg.clone());
        e.set_local_player("epic", EPIC, "Me");
        e.inject_player(PlayerId::new(Platform::Epic, EPIC), Some("Me".into()), Some(Team::Blue));
        e.inject_player(id(1), Some("Mate".into()), Some(Team::Blue));
        e.inject_player(id(2), Some("Foe".into()), Some(Team::Orange));
        assert_eq!(e.snapshot().players.len(), 2);
        cfg.filters.only_opponents = true;
        e.update_config(cfg);
        let snap = e.snapshot();
        assert_eq!(snap.players.len(), 1);
        assert_eq!(snap.players[0].name, "Foe");
    }

    #[test]
    fn local_player_is_excluded_by_id_and_by_name() {
        let e = engine(Config::default());
        assert!(!e.set_local_player("epic", "tooshort", "Bad"), "identifiant invalide refusé");
        assert!(e.set_local_player("epic", EPIC, "neyor."));
        assert_eq!(e.snapshot().me.as_deref(), Some("neyor. · epic"));
        e.inject_player(PlayerId::new(Platform::Epic, EPIC), None, Some(Team::Blue));
        e.inject_player(id(7), Some("neyor.".into()), Some(Team::Blue));
        assert!(e.snapshot().players.is_empty());
    }

    #[test]
    fn new_game_launch_replaces_the_account() {
        let e = engine(Config::default());
        e.set_local_player("epic", EPIC, "neyor.");
        e.inject_player(id(1), None, None);
        e.game_launched();
        let snap = e.snapshot();
        assert_eq!(snap.me, None, "le relancement efface l'ancien compte");
        assert!(snap.players.is_empty());
        assert!(e.set_local_player("steam", "76561198012345678", "SteamMe"));
        assert_eq!(e.snapshot().me.as_deref(), Some("SteamMe · steam"));
    }

    #[test]
    fn stats_api_events_fill_the_lobby_and_signal_kickoff() {
        let e = engine(Config::default());
        let mut kickoffs = e.subscribe_kickoff();
        e.ingest_stats(
            "UpdateState",
            &json!({"Players":[{"Name":"Zed","PrimaryId":"Steam|76561198000000001|0","TeamNum":1}]}),
        );
        let snap = e.snapshot();
        assert_eq!(snap.players.len(), 1);
        assert_eq!(snap.players[0].name, "Zed");
        e.ingest_stats("RoundStarted", &json!({}));
        assert!(kickoffs.try_recv().is_ok());
        e.ingest_stats("MatchCreated", &json!({}));
        assert!(e.snapshot().players.is_empty());
    }

    #[test]
    fn preview_shows_demo_badges_respects_filters_and_restores_real_lobby() {
        let mut cfg = Config::default();
        let e = engine(cfg.clone());
        e.inject_player(id(1), Some("Réel".into()), None);
        e.set_preview(true);
        let snap = e.snapshot();
        assert!(snap.players.iter().any(|p| p.name == "BumpMachine"));
        let all: Vec<_> = snap.players.iter().flat_map(|p| p.badge_labels.clone()).collect();
        for l in ["Potentiel smurf", "Joueur 1v1", "Gardien", "Lose streak", "Ancien haut rang"] {
            let l = l.to_string();
            assert!(all.iter().any(|x| x.starts_with(&l)), "badge démo manquant : {l}");
        }
        cfg.filters.on_tilt = false;
        e.update_config(cfg);
        assert!(!e.snapshot().players.iter().any(|p| p.badge_labels.iter().any(|l| l.starts_with("Lose streak"))));
        e.set_preview(false);
        let real = e.snapshot();
        assert_eq!(real.players.len(), 1);
        assert_eq!(real.players[0].name, "Réel");
    }
}
