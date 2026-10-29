//! Intégration de LobbyTracker : moteur d'analyse du lobby, overlay dédié et commandes de l'onglet.
//!
//! Tout ce qui existait déjà dans RL Tracker Suite est réutilisé : la connexion à la Stats API
//! (`stats_client`), la détection du jeu et du joueur, l'accès à Tracker Network (`trn_web`),
//! le serveur OBS (`web_server`), le stockage des paramètres (`settings`) et le placement des
//! fenêtres d'overlay (`show_overlay_window`).

use crate::{drag_window_by, park_overlay_window, show_overlay_window, window_position_on_screen, Ctx};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tracker_core::{AutoHideMode, Config};
use tracker_engine::Engine;
use tracker_scraper::{ScrapeError, WebFetch};

pub const OVERLAY: &str = "lt-overlay";
const PREVIEW_DURATION: Duration = Duration::from_secs(30);

/// Interroge Tracker Network avec la WebView masquée de l'application.
struct AppWebFetch(AppHandle);

#[async_trait]
impl WebFetch for AppWebFetch {
    async fn get_json(&self, api_url: &str) -> Result<Value, ScrapeError> {
        crate::trn_web::fetch_json(&self.0, api_url).await.map_err(|e| {
            if e.contains("introuvable") {
                ScrapeError::NotFound
            } else if e.contains("429") {
                ScrapeError::RateLimited
            } else if e.contains("Délai") {
                ScrapeError::Timeout
            } else {
                ScrapeError::Webview(e)
            }
        })
    }
}

pub struct LobbyState {
    engine: Arc<Engine>,
    /// L'overlay est actuellement affiché à l'écran (et non rangé hors écran).
    visible: AtomicBool,
    /// Vrai tant que le lobby contient au moins un joueur.
    has_players: AtomicBool,
    preview: AtomicBool,
    preview_generation: AtomicU64,
    size: Mutex<(f64, f64)>,
    autohide_generation: AtomicU64,
    hidden_by_autohide: AtomicBool,
    awaiting_kickoff: AtomicBool,
}

pub fn create_engine(app: &AppHandle, cfg: Config, cache_path: &Path) -> Result<Arc<Engine>, String> {
    Engine::new(cfg, cache_path, Some(Arc::new(AppWebFetch(app.clone())))).map_err(|e| e.to_string())
}

/// État envoyé à l'overlay et à la source OBS : le lobby analysé et les réglages d'affichage.
pub fn state_value(engine: &Engine) -> Value {
    json!({ "snapshot": engine.snapshot(), "display": engine.config() })
}

fn lobby(app: &AppHandle) -> tauri::State<'_, LobbyState> {
    app.state::<LobbyState>()
}

/// Enregistre l'état LobbyTracker et lance le moteur et les ponts d'événements.
pub fn start(app: &AppHandle, engine: Arc<Engine>) {
    app.manage(LobbyState {
        engine: engine.clone(),
        visible: AtomicBool::new(false),
        has_players: AtomicBool::new(false),
        preview: AtomicBool::new(false),
        preview_generation: AtomicU64::new(0),
        size: Mutex::new((380.0, 200.0)),
        autohide_generation: AtomicU64::new(0),
        hidden_by_autohide: AtomicBool::new(false),
        awaiting_kickoff: AtomicBool::new(false),
    });

    let mut snapshots = engine.subscribe();
    let mut kickoffs = engine.subscribe_kickoff();

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        engine.start().await;
        let mut had_players = false;
        loop {
            match snapshots.recv().await {
                Ok(snap) => {
                    let has_players = !snap.players.is_empty();
                    lobby(&handle).has_players.store(has_players, Ordering::Relaxed);
                    if has_players && !had_players {
                        on_new_lobby(&handle);
                    }
                    had_players = has_players;
                    let state = json!({ "snapshot": snap, "display": lobby(&handle).engine.config() });
                    let _ = handle.emit("lt-state", &state);
                    handle.state::<Ctx>().web.broadcast_lobby(&state);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match kickoffs.recv().await {
                Ok(()) => on_kickoff(&handle),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Événements venant du reste de l'application
// ---------------------------------------------------------------------------

/// Événement de la Stats API, déjà décodé par `stats_client`.
pub fn on_stats_event(app: &AppHandle, name: &str, data: &Value) {
    lobby(app).engine.ingest_stats(name, data);
}

/// Rocket League vient d'être lancé : l'ancien compte et le lobby sont oubliés.
pub fn on_game_launched(app: &AppHandle) {
    lobby(app).engine.game_launched();
}

/// Compte local détecté par RL Tracker Suite dans `Launch.log`.
pub fn on_local_player(app: &AppHandle, platform: &str, id: &str, name: &str) {
    lobby(app).engine.set_local_player(platform, id, name);
}

// ---------------------------------------------------------------------------
// Masquage automatique
// ---------------------------------------------------------------------------

/// Un nouveau lobby vient de se remplir : l'overlay s'affiche, puis se masque selon le mode choisi.
fn on_new_lobby(app: &AppHandle) {
    let st = lobby(app);
    let cfg = st.engine.config();
    st.hidden_by_autohide.store(false, Ordering::Relaxed);
    if cfg.auto_hide_mode == AutoHideMode::Off || st.preview.load(Ordering::Relaxed) {
        return;
    }
    let generation = st.autohide_generation.fetch_add(1, Ordering::SeqCst) + 1;
    st.awaiting_kickoff.store(cfg.auto_hide_mode == AutoHideMode::Kickoff, Ordering::SeqCst);
    if cfg.auto_hide_mode != AutoHideMode::Timer {
        return;
    }
    let app = app.clone();
    let delay = Duration::from_secs(u64::from(cfg.auto_hide_seconds.clamp(5, 600)));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let st = lobby(&app);
        if st.autohide_generation.load(Ordering::SeqCst) == generation && !st.preview.load(Ordering::Relaxed) {
            st.hidden_by_autohide.store(true, Ordering::Relaxed);
        }
    });
}

/// Coup d'envoi (fin du décompte) : masque l'overlay tout de suite, une seule fois par lobby.
fn on_kickoff(app: &AppHandle) {
    let st = lobby(app);
    if st.engine.config().auto_hide_mode != AutoHideMode::Kickoff || st.preview.load(Ordering::Relaxed) {
        return;
    }
    if !st.awaiting_kickoff.swap(false, Ordering::SeqCst) {
        return;
    }
    tracing_log(app, "coup d'envoi détecté : overlay du lobby masqué");
    st.autohide_generation.fetch_add(1, Ordering::SeqCst);
    st.hidden_by_autohide.store(true, Ordering::Relaxed);
}

fn tracing_log(app: &AppHandle, message: &str) {
    app.state::<Ctx>().core.lock().unwrap().log(message);
}

// ---------------------------------------------------------------------------
// Fenêtre d'overlay
// ---------------------------------------------------------------------------

fn render_overlay(app: &AppHandle) {
    let st = lobby(app);
    let Some(win) = app.get_webview_window(OVERLAY) else { return };
    let cfg = st.engine.config();
    let size = *st.size.lock().unwrap();
    let preview = st.preview.load(Ordering::Relaxed);
    let shown = show_overlay_window(
        &win,
        size,
        &cfg.overlay_position,
        (cfg.overlay_offset_x, cfg.overlay_offset_y),
        (cfg.overlay_custom_x, cfg.overlay_custom_y),
        preview,
    );
    if shown {
        st.visible.store(true, Ordering::Relaxed);
    }
}

fn hide_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY) {
        park_overlay_window(&win);
    }
    lobby(app).visible.store(false, Ordering::Relaxed);
}

/// Appelé toutes les 250 ms par la veille du jeu : affiche ou range l'overlay du lobby.
/// `in_game` = Rocket League est au premier plan (ou simplement lancé selon le réglage).
pub fn tick(app: &AppHandle, in_game: bool) {
    let st = lobby(app);
    let cfg = st.engine.config();
    let preview = st.preview.load(Ordering::Relaxed);
    let should_show = preview
        || (cfg.overlay_enabled
            && in_game
            && st.has_players.load(Ordering::Relaxed)
            && !st.hidden_by_autohide.load(Ordering::Relaxed));
    if should_show {
        if !st.visible.load(Ordering::Relaxed) {
            render_overlay(app);
        } else if let Some(win) = app.get_webview_window(OVERLAY) {
            let _ = win.set_always_on_top(true);
        }
    } else if st.visible.load(Ordering::Relaxed) {
        hide_overlay(app);
    }
}

/// Raccourci global : affiche ou masque l'overlay du lobby.
pub fn toggle_overlay(app: &AppHandle) {
    let mut cfg = lobby(app).engine.config();
    cfg.overlay_enabled = !cfg.overlay_enabled;
    let _ = apply_config(app, cfg);
}

/// Enregistre la configuration dans le fichier de paramètres puis l'applique à chaud.
fn apply_config(app: &AppHandle, cfg: Config) -> Config {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        c.store.update(json!({ "lobbytracker": &cfg }));
    }
    let st = lobby(app);
    st.engine.update_config(cfg.clone());
    if st.visible.load(Ordering::Relaxed) {
        render_overlay(app);
    }
    cfg
}

// ---------------------------------------------------------------------------
// Commandes appelées par l'interface
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn lt_state(app: AppHandle) -> Value {
    state_value(&lobby(&app).engine)
}

#[tauri::command]
pub fn lt_save_config(app: AppHandle, config: Config) -> Config {
    apply_config(&app, config)
}

/// Affiche (ou arrête) le lobby fictif sur l'overlay, déplaçable à la souris pendant l'aperçu.
#[tauri::command]
pub fn lt_set_preview(app: AppHandle, preview: bool) -> bool {
    let st = lobby(&app);
    let generation = st.preview_generation.fetch_add(1, Ordering::SeqCst) + 1;
    st.preview.store(preview, Ordering::Relaxed);
    st.engine.set_preview(preview);
    if preview {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(PREVIEW_DURATION).await;
            let st = lobby(&app2);
            if st.preview_generation.load(Ordering::SeqCst) == generation && st.preview.swap(false, Ordering::Relaxed) {
                st.engine.set_preview(false);
                let _ = app2.emit("lt-preview-ended", ());
            }
        });
        render_overlay(&app);
    }
    preview
}

/// L'overlay mesure son contenu ; la fenêtre suit cette taille.
#[tauri::command]
pub fn lt_overlay_size(app: AppHandle, w: f64, h: f64) {
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return;
    }
    let st = lobby(&app);
    let (w, h) = (w.ceil(), h.ceil());
    {
        let mut size = st.size.lock().unwrap();
        if (size.0 - w).abs() < 2.0 && (size.1 - h).abs() < 2.0 {
            return;
        }
        *size = (w, h);
    }
    if st.visible.load(Ordering::Relaxed) {
        render_overlay(&app);
    }
}

#[tauri::command]
pub fn lt_drag_by(app: AppHandle, dx: f64, dy: f64) {
    drag_window_by(&app, OVERLAY, dx, dy);
}

/// Fin d'un glisser-déposer : l'emplacement devient « libre » et est enregistré.
#[tauri::command]
pub fn lt_drag_finish(app: AppHandle) {
    let Some((x, y)) = window_position_on_screen(&app, OVERLAY) else { return };
    let mut cfg = lobby(&app).engine.config();
    cfg.overlay_position = "custom".into();
    cfg.overlay_custom_x = Some(x);
    cfg.overlay_custom_y = Some(y);
    apply_config(&app, cfg);
}
