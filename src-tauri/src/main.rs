// Pas de console noire au lancement en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rl_config;
mod session;
mod settings;
mod lobby;
mod stats_client;
mod trn_web;
mod web_server;

#[cfg(test)]
mod tests;

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry,
};
use tauri_plugin_dialog::DialogExt;

use session::{calculate_rank, now_ms, CountRules, Effects, Session, Tracker, PLAYLISTS, RANKED};
use settings::SettingsStore;
use stats_client::ClientEvent;
use web_server::WebServer;

const OFFSCREEN: f64 = -32000.0;

// ---------------------------------------------------------------------------
// État partagé
// ---------------------------------------------------------------------------

struct Core {
    store: SettingsStore,
    tracker: Tracker,
    connected: bool,
    overlay_size: (f64, f64),
    obs_dir: PathBuf,
    data_dir: PathBuf,
    session_file: PathBuf,
    history_file: PathBuf,
    log_file: PathBuf,
    documents_dir: Option<PathBuf>,
    current_player_key: String,
}

fn sanitize_account_key(platform: &str, name: &str) -> String {
    let p = platform.trim().to_lowercase();
    let n: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if n.is_empty() {
        "default".to_string()
    } else if p.is_empty() {
        n
    } else {
        format!("{p}_{n}")
    }
}

impl Core {
    fn rules(&self) -> CountRules {
        let c = &self.store.data.counting;
        let o = &self.store.data.overlay;
        CountRules {
            online_only: c.online_only,
            mode: c.mode.clone(),
            playlists: c.playlists.clone(),
            count_leave_as_loss: c.count_leave_as_loss,
            mmr_delta: if o.mmr_delta > 0.0 { o.mmr_delta } else { 9.0 },
            focused_playlist: c.focused_playlist.clone(),
            auto_switch_playlist: c.auto_switch_playlist,
        }
    }

    fn identity(&self) -> Option<String> {
        let p = &self.store.data.player;
        if !p.custom_name.is_empty() {
            Some(format!("NAME:{}|{}", p.platform, p.custom_name))
        } else {
            let id = if p.primary_id.is_empty() { &p.detected_id } else { &p.primary_id };
            (!id.is_empty()).then(|| id.clone())
        }
    }

    fn log(&self, message: &str) {
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.log_file) {
            let _ = writeln!(f, "{message}");
        }
    }

    fn overlay_state(&self) -> Value {
        json!({
            "connected": self.connected,
            "inMatch": self.tracker.in_match(),
            "session": self.tracker.session,
            "display": self.store.data.overlay,
        })
    }

    fn load_history(&self) -> Vec<Value> {
        fs::read_to_string(&self.history_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
            .unwrap_or_default()
    }

    fn switch_account_if_needed(&mut self, new_platform: &str, new_name: &str, new_id: &str) -> bool {
        if new_name.trim().is_empty() {
            return false;
        }
        let new_key = sanitize_account_key(new_platform, new_name);
        if self.current_player_key == new_key {
            return false;
        }

        // Si une session avec des parties était en cours sur le compte précédent, on l'archive
        if self.tracker.session.games > 0 {
            self.archive_session(&self.tracker.session.clone());
        }

        // Sauvegarde de l'état de session sous l'ancienne clé
        if !self.current_player_key.is_empty() {
            let prev_file = self.data_dir.join(format!("session_{}.json", self.current_player_key));
            if let Ok(text) = serde_json::to_string_pretty(&self.tracker.session) {
                let _ = fs::write(&prev_file, text);
            }
        }

        self.current_player_key = new_key.clone();
        self.session_file = self.data_dir.join(format!("session_{new_key}.json"));

        // Chargement de la session spécifique à ce compte ou réinitialisation propre
        let loaded = fs::read_to_string(&self.session_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Session>(&t).ok());

        if let Some(s) = loaded {
            self.tracker.session = s;
        } else {
            let mut s = Session::default();
            s.focused_playlist = self.store.data.counting.focused_playlist.clone();
            let (name, div, tier) = calculate_rank(s.current_mmr, &s.focused_playlist);
            s.rank_name = name.to_string();
            s.division = div;
            s.rank_tier = tier;
            self.tracker.session = s;
        }

        self.store.update(json!({
            "player": {
                "platform": new_platform,
                "customName": new_name,
                "detectedName": new_name,
                "detectedId": new_id
            }
        }));

        self.persist_session();
        self.log(&format!("Changement de compte détecté : passage sur {new_name} ({new_platform})"));
        true
    }

    fn archive_session(&self, s: &Session) {
        if s.games == 0 {
            return;
        }
        let mut list = self.load_history();
        let started_at = s.started_at;
        let ended_at = now_ms();
        let p = &self.store.data.player;
        let player_name = if !p.custom_name.is_empty() {
            p.custom_name.clone()
        } else if !p.detected_name.is_empty() {
            p.detected_name.clone()
        } else {
            "Joueur".to_string()
        };
        let platform = if !p.platform.is_empty() { p.platform.clone() } else { "epic".to_string() };
        let record = json!({
            "id": format!("{started_at}"),
            "startedAt": started_at,
            "endedAt": ended_at,
            "games": s.games,
            "wins": s.wins,
            "losses": s.losses,
            "streak": s.streak,
            "bestStreak": s.best_streak,
            "sessionMmrChange": s.session_mmr_change,
            "currentMmr": s.current_mmr,
            "startingMmr": s.current_mmr - s.session_mmr_change,
            "rankName": s.rank_name,
            "division": s.division,
            "rankTier": s.rank_tier,
            "focusedPlaylist": s.focused_playlist,
            "recent": s.recent,
            "playlists": s.playlists,
            "playerName": player_name,
            "platform": platform,
            "playerId": p.detected_id.clone()
        });

        if let Some(pos) = list.iter().position(|item| item["id"].as_str() == Some(&format!("{started_at}"))) {
            list[pos] = record;
        } else {
            list.insert(0, record);
        }

        if list.len() > 60 {
            list.truncate(60);
        }

        if let Ok(text) = serde_json::to_string_pretty(&list) {
            let _ = fs::write(&self.history_file, text);
        }
    }

    fn delete_history_item(&self, id: &str) {
        let mut list = self.load_history();
        list.retain(|item| item["id"].as_str() != Some(id));
        if let Ok(text) = serde_json::to_string_pretty(&list) {
            let _ = fs::write(&self.history_file, text);
        }
    }

    fn full_state(&self, server_error: Option<String>) -> Value {
        let playlists: serde_json::Map<String, Value> =
            PLAYLISTS.iter().map(|(id, name)| (id.to_string(), json!(name))).collect();
        let history = self.load_history();
        json!({
            "settings": self.store.data,
            "session": self.tracker.session,
            "history": history,
            "connected": self.connected,
            "inMatch": self.tracker.in_match(),
            "playlists": playlists,
            "ranked": RANKED,
            "obsUrl": format!("http://localhost:{}/overlay", self.store.data.server.port),
            "lobbyObsUrl": format!("http://localhost:{}/lobby", self.store.data.server.port),
            "obsDir": self.obs_dir,
            "serverError": server_error,
            "shortcut": "Ctrl+Maj+O",
        })
    }

    fn tooltip(&self) -> String {
        let s = &self.tracker.session;
        format!("RL Tracker Suite - {}V / {}D - streak {} - MMR {:.1}", s.wins, s.losses, fmt_streak(s.streak), s.current_mmr)
    }

    /// Sauvegarde la session et écrit les fichiers texte pour OBS.
    fn persist_session(&self) {
        let s = &self.tracker.session;
        if let Ok(text) = serde_json::to_string_pretty(s) {
            let _ = fs::write(&self.session_file, text);
        }
        if s.games > 0 {
            self.archive_session(s);
        }
        let _ = fs::create_dir_all(&self.obs_dir);
        let winrate = if s.games > 0 { format!("{}%", (100 * s.wins + s.games / 2) / s.games) } else { "-".into() };
        let mmr_str = format!("{:.1}", s.current_mmr);
        let mmr_change_str = format!("{:+.1}", s.last_match_mmr_change);
        let mmr_session_str = format!("{:+.1}", s.session_mmr_change);
        let rank_full = format!("{} Div. {}", s.rank_name, s.division);
        let files = [
            ("games", s.games.to_string()),
            ("wins", s.wins.to_string()),
            ("losses", s.losses.to_string()),
            ("streak", fmt_streak(s.streak)),
            ("winrate", winrate),
            ("best", fmt_streak(s.best_streak)),
            ("goals", s.totals.goals.to_string()),
            ("assists", s.totals.assists.to_string()),
            ("saves", s.totals.saves.to_string()),
            ("shots", s.totals.shots.to_string()),
            ("demos", s.totals.demos.to_string()),
            ("deaths", s.totals.deaths.to_string()),
            ("mmr", mmr_str),
            ("mmr_change", mmr_change_str),
            ("mmr_session", mmr_session_str),
            ("rank", s.rank_name.clone()),
            ("division", format!("Div. {}", s.division)),
            ("rank_full", rank_full),
            ("mode", s.focused_playlist.clone()),
        ];
        for (name, value) in files {
            let _ = fs::write(self.obs_dir.join(format!("{name}.txt")), value);
        }
    }
}

fn fmt_streak(n: i64) -> String {
    if n > 0 { format!("+{n}") } else { n.to_string() }
}

struct Ctx {
    core: Arc<Mutex<Core>>,
    rl_port: Arc<AtomicU16>,
    web: WebServer,
    toggle_item: Mutex<Option<CheckMenuItem<Wry>>>,
    preview_overlay: Arc<AtomicBool>,
    overlay_visible: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Diffusion de l'état (fenêtres, OBS, icône)
// ---------------------------------------------------------------------------

fn publish(app: &AppHandle) {
    let ctx = app.state::<Ctx>();
    let (full, overlay, tooltip, enabled) = {
        let c = ctx.core.lock().unwrap();
        (c.full_state(ctx.web.error()), c.overlay_state(), c.tooltip(), c.store.data.overlay.enabled)
    };
    let _ = app.emit("state", &full);
    let _ = app.emit("overlay", &overlay);
    ctx.web.broadcast(&overlay);
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip));
    }
    let item = ctx.toggle_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_checked(enabled);
    }
}

fn after_effects(app: &AppHandle, effects: Effects) {
    let ctx = app.state::<Ctx>();
    {
        let mut c = ctx.core.lock().unwrap();
        for line in &effects.logs {
            c.log(line);
        }
        if let Some((id, name)) = &effects.detected {
            let p = &c.store.data.player;
            let current_name = if !p.custom_name.trim().is_empty() {
                p.custom_name.trim()
            } else if !p.primary_id.trim().is_empty() {
                p.primary_id.trim()
            } else {
                p.detected_name.trim()
            };
            if !name.trim().is_empty() && (current_name.is_empty() || !current_name.eq_ignore_ascii_case(name.trim())) {
                let plat = c.store.data.player.platform.clone();
                c.switch_account_if_needed(&plat, name, id);
                c.log(&format!("Profil joueur synchronisé via la caméra de match : {} ({})", name, plat));
            } else if p.detected_id != *id || p.detected_name != *name {
                c.store.update(json!({ "player": { "detectedId": id, "detectedName": name } }));
            }
        }
        if effects.session_changed {
            c.persist_session();
            let auto_sync = c.store.data.player.auto_sync_mmr;
            if auto_sync {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    std::thread::sleep(Duration::from_secs(4));
                    let _ = perform_tracker_sync(app_handle.clone()).await;
                    std::thread::sleep(Duration::from_secs(6));
                    let _ = perform_tracker_sync(app_handle).await;
                });
            }
        }
    }
    if effects.session_changed || effects.status_changed || effects.detected.is_some() {
        publish(app);
    }
}

// ---------------------------------------------------------------------------
// Overlay en jeu
// ---------------------------------------------------------------------------

/// Premier lancement : reprend réglages, historique et sessions de l'ancienne application
/// « RL Session Stats » (dossier `fr.rlsessionstats.app`) si aucun réglage n'existe encore ici.
/// Renvoie le nombre de fichiers repris.
pub(crate) fn import_legacy_data(new_dir: &std::path::Path, old_dir: &std::path::Path) -> usize {
    if new_dir.join("settings.json").exists() || !old_dir.is_dir() {
        return 0;
    }
    let mut copied = 0;
    if let Ok(entries) = fs::read_dir(old_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let wanted = name == "settings.json" || name == "history.json" || (name.starts_with("session") && name.ends_with(".json"));
            if wanted && fs::copy(entry.path(), new_dir.join(&name)).is_ok() {
                copied += 1;
            }
        }
    }
    copied
}

/// Coin haut-gauche d'une fenêtre d'overlay (pixels logiques) selon l'emplacement choisi.
/// Partagé par l'overlay de session et l'overlay du lobby.
pub(crate) fn overlay_origin(
    monitor_pos: (f64, f64),
    monitor_size: (f64, f64),
    win: (f64, f64),
    position: &str,
    offset: (f64, f64),
    custom: (Option<f64>, Option<f64>),
) -> (f64, f64) {
    let ((px, py), (mw, mh), (w, h), (ox, oy)) = (monitor_pos, monitor_size, win, offset);
    if position == "custom" {
        return (px + custom.0.unwrap_or(ox), py + custom.1.unwrap_or(oy));
    }
    let (vertical, horizontal) = position.split_once('-').unwrap_or(("top", "left"));
    let x = match horizontal {
        "left" => px + ox,
        "right" => px + mw - w - ox,
        _ => px + (mw - w) / 2.0 + ox,
    };
    let y = if vertical == "bottom" { py + mh - h - oy } else { py + oy };
    (x, y)
}

/// Place une fenêtre d'overlay à l'écran. `interactive` = elle reçoit la souris (prévisualisation,
/// pour la déplacer) ; sinon les clics la traversent. Renvoie `false` sans écran principal.
pub(crate) fn show_overlay_window(
    win: &tauri::WebviewWindow,
    size: (f64, f64),
    position: &str,
    offset: (f64, f64),
    custom: (Option<f64>, Option<f64>),
    interactive: bool,
) -> bool {
    let Some(monitor) = win.primary_monitor().ok().flatten() else { return false };
    let sf = monitor.scale_factor();
    let pos = monitor.position().to_logical::<f64>(sf);
    let msize = monitor.size().to_logical::<f64>(sf);
    let (w, h) = (size.0.max(40.0), size.1.max(20.0));
    let (x, y) = overlay_origin((pos.x, pos.y), (msize.width, msize.height), (w, h), position, offset, custom);
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(LogicalPosition::new(x.round(), y.round()));
    let _ = win.set_always_on_top(true);
    let _ = win.set_ignore_cursor_events(!interactive);
    true
}

/// Range l'overlay hors écran (la fenêtre reste vivante, sans clignoter à la réouverture).
pub(crate) fn park_overlay_window(win: &tauri::WebviewWindow) {
    let _ = win.set_position(LogicalPosition::new(OFFSCREEN, OFFSCREEN));
    let _ = win.set_ignore_cursor_events(true);
}

/// Déplace une fenêtre d'overlay de (dx, dy) pixels logiques.
pub(crate) fn drag_window_by(app: &AppHandle, label: &str, dx: f64, dy: f64) {
    let Some(win) = app.get_webview_window(label) else { return };
    if let Ok(pos) = win.outer_position() {
        let sf = win.scale_factor().unwrap_or(1.0);
        let _ = win.set_position(LogicalPosition::new(pos.x as f64 / sf + dx, pos.y as f64 / sf + dy));
    }
}

/// Position d'une fenêtre relative à l'écran principal, en pixels logiques (fin d'un glisser).
pub(crate) fn window_position_on_screen(app: &AppHandle, label: &str) -> Option<(f64, f64)> {
    let win = app.get_webview_window(label)?;
    let sf = win.scale_factor().unwrap_or(1.0);
    let (win_pos, mon) = (win.outer_position().ok()?, win.primary_monitor().ok()??);
    let mon_pos = mon.position().to_logical::<f64>(sf);
    let cur = win_pos.to_logical::<f64>(sf);
    Some(((cur.x - mon_pos.x).round(), (cur.y - mon_pos.y).round()))
}

fn render_overlay_on_screen(app: &AppHandle) {
    let ctx = app.state::<Ctx>();
    let Some(win) = app.get_webview_window("overlay") else { return };
    let (o, size, is_preview) = {
        let c = ctx.core.lock().unwrap();
        (c.store.data.overlay.clone(), c.overlay_size, ctx.preview_overlay.load(Ordering::Relaxed))
    };
    if !o.enabled && !is_preview {
        park_overlay_window(&win);
        ctx.overlay_visible.store(false, Ordering::Relaxed);
        return;
    }
    let shown = show_overlay_window(
        &win,
        size,
        &o.position,
        (o.offset_x, o.offset_y),
        (o.custom_x, o.custom_y),
        is_preview,
    );
    if shown {
        ctx.overlay_visible.store(true, Ordering::Relaxed);
    }
}

fn hide_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        park_overlay_window(&win);
    }
    let ctx = app.state::<Ctx>();
    ctx.overlay_visible.store(false, Ordering::Relaxed);
}

fn apply_overlay(app: &AppHandle) {
    let (enabled, only_in_game, is_preview) = {
        let ctx = app.state::<Ctx>();
        let c = ctx.core.lock().unwrap();
        (c.store.data.overlay.enabled, c.store.data.overlay.only_in_game, ctx.preview_overlay.load(Ordering::Relaxed))
    };
    if is_preview {
        render_overlay_on_screen(app);
        return;
    }
    if !enabled {
        hide_overlay(app);
        return;
    }
    if only_in_game && !rl_config::is_rocket_league_focused() {
        hide_overlay(app);
    } else {
        render_overlay_on_screen(app);
    }
}

fn toggle_overlay(app: &AppHandle) {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        let enabled = !c.store.data.overlay.enabled;
        c.store.update(json!({ "overlay": { "enabled": enabled } }));
    }
    apply_overlay(app);
    publish(app);
}

fn open_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn reset_session(app: &AppHandle) {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        if c.tracker.session.games > 0 {
            c.archive_session(&c.tracker.session.clone());
        }
        c.tracker.reset();
        c.persist_session();
        c.log("Session réinitialisée");
    }
    publish(app);
}

// ---------------------------------------------------------------------------
// Commandes appelées par l'interface
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_state(app: AppHandle) -> Value {
    let ctx = app.state::<Ctx>();
    let c = ctx.core.lock().unwrap();
    c.full_state(ctx.web.error())
}

#[tauri::command]
fn save_settings(app: AppHandle, patch: Value) -> Value {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        c.store.update(patch);
        let focus = c.store.data.counting.focused_playlist.clone();
        c.tracker.update_focus(&focus);
        ctx.rl_port.store(c.store.data.stats_api.port, Ordering::SeqCst);
        c.persist_session();
    }
    apply_overlay(&app);
    publish(&app);
    get_state(app)
}

#[tauri::command]
fn reset_session_cmd(app: AppHandle) -> Value {
    reset_session(&app);
    get_state(app)
}

#[tauri::command]
fn set_mmr_cmd(app: AppHandle, mmr: f64) -> Value {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        let focus = c.store.data.counting.focused_playlist.clone();
        c.tracker.set_mmr(mmr, &focus);
        c.store.update(json!({ "overlay": { "startingMmr": mmr } }));
        c.persist_session();
    }
    publish(&app);
    get_state(app)
}

fn stats_api_status(app: &AppHandle) -> Value {
    let (dir, docs) = {
        let ctx = app.state::<Ctx>();
        let c = ctx.core.lock().unwrap();
        (c.store.data.stats_api.install_dir.clone(), c.documents_dir.clone())
    };
    let mut status = rl_config::read_status(&dir, docs.as_deref());
    status["rlRunning"] = json!(rl_config::is_rocket_league_running());
    status
}

#[tauri::command]
async fn statsapi_status(app: AppHandle) -> Value {
    stats_api_status(&app)
}

#[tauri::command]
async fn statsapi_detect(app: AppHandle) -> Value {
    let installs = rl_config::detect_game_installations();
    let target = installs.epic.clone().or(installs.steam.clone());
    if let Some(dir) = target {
        let ctx = app.state::<Ctx>();
        ctx.core.lock().unwrap().store.update(json!({ "statsApi": { "installDir": dir } }));
    }
    publish(&app);
    let mut status = stats_api_status(&app);
    status["notFound"] = json!(installs.epic.is_none() && installs.steam.is_none());
    status
}

#[tauri::command]
async fn statsapi_select_platform(app: AppHandle, platform: String) -> Value {
    let installs = rl_config::detect_game_installations();
    let target = match platform.to_lowercase().as_str() {
        "epic" => installs.epic,
        "steam" => installs.steam,
        _ => None,
    };
    if let Some(dir) = target {
        {
            let ctx = app.state::<Ctx>();
            ctx.core.lock().unwrap().store.update(json!({ "statsApi": { "installDir": dir } }));
        }
        publish(&app);
    }
    stats_api_status(&app)
}

#[tauri::command]
async fn detect_player_now(app: AppHandle) -> Value {
    let detected_plat = if let Some(info) = rl_config::get_running_rocket_league_info() {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        if let Some(ref dir) = info.install_dir {
            c.store.update(json!({ "statsApi": { "installDir": dir } }));
        }
        if info.platform == "epic" || info.platform == "steam" {
            Some(info.platform)
        } else {
            None
        }
    } else {
        None
    };

    let docs = app.state::<Ctx>().core.lock().unwrap().documents_dir.clone();
    if let Some(player) = rl_config::detect_player_from_logs(docs.as_deref()) {
        let final_plat = detected_plat.unwrap_or(player.platform);
        {
            let ctx = app.state::<Ctx>();
            let mut c = ctx.core.lock().unwrap();
            c.switch_account_if_needed(&final_plat, &player.name, &player.id);
            c.log(&format!("Joueur détecté : {} ({})", player.name, final_plat));
        }
        publish(&app);
        let h = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = perform_tracker_sync(h).await;
        });
        json!({
            "success": true,
            "platform": final_plat,
            "name": player.name,
            "id": player.id
        })
    } else {
        json!({
            "success": false,
            "error": "Aucun profil trouv\u{e9} dans les fichiers de log (Launch.log). Lancez Rocket League au moins une fois."
        })
    }
}

#[tauri::command]
async fn statsapi_choose(app: AppHandle) -> Value {
    let picked = app
        .dialog()
        .file()
        .set_title("Choisis le dossier d'installation de Rocket League")
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok());
    let Some(picked) = picked else { return stats_api_status(&app) };

    match rl_config::normalize_install_dir(&picked) {
        Some(dir) => {
            {
                let ctx = app.state::<Ctx>();
                let dir = dir.to_string_lossy().into_owned();
                ctx.core.lock().unwrap().store.update(json!({ "statsApi": { "installDir": dir } }));
            }
            publish(&app);
            stats_api_status(&app)
        }
        None => {
            let mut status = stats_api_status(&app);
            status["error"] = json!("Ce dossier ne contient pas Rocket League (il faut le dossier qui contient « TAGame »).");
            status
        }
    }
}

#[tauri::command]
async fn statsapi_enable(app: AppHandle) -> Value {
    let (dir, docs, rate, port) = {
        let ctx = app.state::<Ctx>();
        let c = ctx.core.lock().unwrap();
        let s = &c.store.data.stats_api;
        (s.install_dir.clone(), c.documents_dir.clone(), s.packet_send_rate, s.port)
    };
    let mut status = stats_api_status(&app);
    if !rl_config::is_valid_install(&dir) {
        status["error"] = json!("Dossier de Rocket League introuvable. Utilise « Détecter » ou « Choisir le dossier ».");
        return status;
    }
    if rl_config::is_rocket_league_running() {
        status["error"] = json!("Ferme complètement Rocket League, puis réessaie : le jeu écrase ce réglage en quittant.");
        return status;
    }
    match rl_config::enable_stats_api(&dir, docs.as_deref(), rate, port) {
        Ok(_) => {
            app.state::<Ctx>().core.lock().unwrap().log("Stats API activée");
            let mut status = stats_api_status(&app);
            status["success"] = json!("Stats API activée. Lance Rocket League : le réglage est lu au démarrage du jeu.");
            status
        }
        Err(err) => {
            status["error"] = json!(if err.kind() == std::io::ErrorKind::PermissionDenied {
                "Windows refuse l'écriture dans le dossier du jeu. Quitte l'appli, fais un clic droit dessus > « Exécuter en tant qu'administrateur », puis réessaie.".to_string()
            } else {
                format!("Impossible de modifier le fichier : {err}")
            });
            status
        }
    }
}

#[tauri::command]
fn open_obs_folder(app: AppHandle) {
    let dir = app.state::<Ctx>().core.lock().unwrap().obs_dir.clone();
    let _ = fs::create_dir_all(&dir);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
}

#[tauri::command]
fn open_browser_url(url: String) {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
}

async fn perform_tracker_sync(app: AppHandle) -> Result<Value, String> {
    let (platform, username, focus) = {
        let ctx = app.state::<Ctx>();
        let c = ctx.core.lock().unwrap();
        let p = &c.store.data.player;
        let mut user = p.custom_name.trim().to_string();
        if user.is_empty() {
            user = p.primary_id.trim().to_string();
        }
        if user.is_empty() {
            user = p.detected_name.trim().to_string();
        }
        let mut plat = p.platform.trim().to_lowercase();
        if plat.is_empty() {
            plat = "epic".to_string();
        }
        (plat, user, c.store.data.counting.focused_playlist.clone())
    };

    if username.is_empty() {
        return Err("Veuillez d'abord renseigner votre pseudo dans l'onglet Joueur.".into());
    }

    let encoded_user: String = username
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            _ => c.to_string(),
        })
        .collect();

    let page_url = format!("https://rocketleague.tracker.network/rocket-league/profile/{platform}/{encoded_user}");
    let api_url = format!("https://api.tracker.gg/api/v2/rocket-league/standard/profile/{platform}/{encoded_user}");
    let json = trn_web::fetch_json(&app, &page_url, &api_url).await?;
    let Some(segments) = json["data"]["segments"].as_array().cloned() else {
        return Err("Profil Tracker Network inattendu : aucune statistique.".into());
    };

    let target_playlist = match focus.as_str() {
        "1v1" => "playlist-10",
        "3v3" => "playlist-13",
        _ => "playlist-11",
    };

    let mut found_segment = None;
    for seg in &segments {
        if seg["attributes"]["playlist"].as_str() == Some(target_playlist) {
            found_segment = Some(seg.clone());
            break;
        }
    }

    if found_segment.is_none() {
        let name_target = match focus.as_str() {
            "1v1" => "Ranked Duel 1v1",
            "3v3" => "Ranked Standard 3v3",
            _ => "Ranked Doubles 2v2",
        };
        for seg in &segments {
            if let Some(name) = seg["metadata"]["name"].as_str() {
                if name.contains(name_target) {
                    found_segment = Some(seg.clone());
                    break;
                }
            }
        }
    }

    let Some(seg) = found_segment else {
        return Err(format!("Aucune statistique classée trouvée pour le mode {focus} sur votre profil."));
    };

    let rating = seg["stats"]["rating"]["value"].as_f64().unwrap_or(0.0);
    let rank_name = seg["stats"]["tier"]["metadata"]["name"].as_str().unwrap_or("Unranked");
    let division_name = seg["stats"]["division"]["metadata"]["name"].as_str().unwrap_or("Division I");

    // Extraction et mise en cache des MMR pour chaque playlist classée disponible sur le profil
    let mode_mappings = [
        ("playlist-10", "Ranked Duel 1v1", "1v1"),
        ("playlist-11", "Ranked Doubles 2v2", "2v2"),
        ("playlist-13", "Ranked Standard 3v3", "3v3"),
        ("playlist-27", "Hoops", "hoops"),
        ("playlist-28", "Rumble", "rumble"),
        ("playlist-29", "Dropshot", "dropshot"),
        ("playlist-30", "Snowday", "snowday"),
        ("playlist-34", "Tournament", "tournaments"),
    ];

    let mut all_ratings = std::collections::BTreeMap::new();
    for s in &segments {
        let seg_attr = s["attributes"]["playlist"].as_str().unwrap_or("");
        let seg_n = s["metadata"]["name"].as_str().unwrap_or("");
        let r = s["stats"]["rating"]["value"].as_f64().unwrap_or(0.0);
        if r > 0.0 {
            for (attr, name_hint, mode) in &mode_mappings {
                if seg_attr == *attr || seg_n.contains(name_hint) {
                    all_ratings.insert(mode.to_string(), r);
                }
            }
        }
    }

    if rating > 0.0 {
        {
            let ctx = app.state::<Ctx>();
            let mut c = ctx.core.lock().unwrap();
            for (m, r) in all_ratings {
                c.tracker.session.playlists_mmr.insert(m, r);
            }
            let is_fresh_session = c.tracker.session.games == 0;
            let old_mmr = c.tracker.session.current_mmr;
            c.tracker.set_mmr(rating, &focus);
            c.tracker.session.playlists_mmr.insert(focus.clone(), rating);
            if is_fresh_session {
                c.tracker.session.session_mmr_change = 0.0;
                c.tracker.session.last_match_mmr_change = 0.0;
            } else if old_mmr > 0.0 {
                let diff = rating - old_mmr;
                c.tracker.session.last_match_mmr_change = diff;
                c.tracker.session.session_mmr_change += diff;
            }
            let start_mmr = if is_fresh_session { rating } else { c.store.data.overlay.starting_mmr };
            c.store.update(json!({ "overlay": { "startingMmr": start_mmr } }));
            c.persist_session();
            c.log(&format!("MMR synchronisé en direct depuis Tracker Network : {:.1} ({} {})", rating, rank_name, division_name));
        }
        publish(&app);
    }

    Ok(json!({
        "success": true,
        "mmr": rating,
        "rank": rank_name,
        "division": division_name,
        "playlist": focus
    }))
}

#[tauri::command]
async fn sync_tracker_mmr(app: AppHandle) -> Result<Value, String> {
    perform_tracker_sync(app).await
}

#[tauri::command]
fn get_autostart() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("reg");
        cmd.args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "RL Tracker Suite"]);
        cmd.creation_flags(0x0800_0000);
        let output = cmd.output();
        output.map(|o| o.status.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    false
}

#[tauri::command]
fn set_autostart(enabled: bool) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if enabled {
            if let Ok(exe_path) = std::env::current_exe() {
                let cmd = format!("\"{}\" --minimized", exe_path.to_string_lossy());
                let mut p = std::process::Command::new("reg");
                p.args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "RL Tracker Suite", "/t", "REG_SZ", "/d", &cmd, "/f"]);
                p.creation_flags(0x0800_0000);
                let output = p.output();
                return output.map(|o| o.status.success()).unwrap_or(false);
            }
        } else {
            let mut p = std::process::Command::new("reg");
            p.args(["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "RL Tracker Suite", "/f"]);
            p.creation_flags(0x0800_0000);
            let output = p.output();
            return output.map(|o| o.status.success()).unwrap_or(false);
        }
    }
    false
}

fn handle_exit_cleanup(app: &AppHandle) {
    let ctx = app.state::<Ctx>();
    let mut c = ctx.core.lock().unwrap();
    if c.tracker.session.games > 0 {
        c.archive_session(&c.tracker.session.clone());
    }
    if c.store.data.counting.reset_on_exit {
        c.tracker.reset();
        c.persist_session();
    }
}

#[tauri::command]
fn delete_history_session(app: AppHandle, id: String) -> Value {
    {
        let ctx = app.state::<Ctx>();
        let c = ctx.core.lock().unwrap();
        c.delete_history_item(&id);
    }
    publish(&app);
    get_state(app)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    handle_exit_cleanup(&app);
    app.exit(0);
}

#[tauri::command]
fn overlay_state(app: AppHandle) -> Value {
    app.state::<Ctx>().core.lock().unwrap().overlay_state()
}

#[tauri::command]
fn overlay_size(app: AppHandle, w: f64, h: f64) {
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return;
    }
    let (changed, final_w, final_h) = {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        let (cur_w, cur_h) = c.overlay_size;
        let w = w.ceil();
        let h = h.ceil();
        if (cur_w - w).abs() < 2.0 && (cur_h - h).abs() < 2.0 {
            (false, cur_w, cur_h)
        } else {
            c.overlay_size = (w, h);
            (true, w, h)
        }
    };
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.set_size(LogicalSize::new(final_w, final_h));
    }
    if changed {
        apply_overlay(&app);
    }
}

#[tauri::command]
fn drag_overlay_by(app: AppHandle, dx: f64, dy: f64) {
    drag_window_by(&app, "overlay", dx, dy);
}

#[tauri::command]
fn save_overlay_drag_finish(app: AppHandle) {
    let Some((rel_x, rel_y)) = window_position_on_screen(&app, "overlay") else { return };
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        c.store.update(json!({
            "overlay": {
                "position": "custom",
                "customX": rel_x,
                "customY": rel_y
            }
        }));
        c.store.save();
    }
    publish(&app);
}

#[tauri::command]
fn set_overlay_preview(app: AppHandle, preview: bool) -> bool {
    let ctx = app.state::<Ctx>();
    ctx.preview_overlay.store(preview, Ordering::Relaxed);
    apply_overlay(&app);
    preview
}

#[tauri::command]
fn get_overlay_preview(app: AppHandle) -> bool {
    let ctx = app.state::<Ctx>();
    ctx.preview_overlay.load(Ordering::Relaxed)
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn set_overlay_custom_position(app: AppHandle, x: f64, y: f64) {
    {
        let ctx = app.state::<Ctx>();
        let mut c = ctx.core.lock().unwrap();
        c.store.update(json!({
            "overlay": {
                "position": "custom",
                "customX": x.round(),
                "customY": y.round()
            }
        }));
        c.store.save();
    }
    apply_overlay(&app);
    publish(&app);
}

// ---------------------------------------------------------------------------
// Démarrage
// ---------------------------------------------------------------------------

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    let data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&data_dir)?;
    let log_file = data_dir.join("journal.txt");
    let _ = fs::write(&log_file, "");

    if let Some(parent) = data_dir.parent() {
        import_legacy_data(&data_dir, &parent.join("fr.rlsessionstats.app"));
    }
    let mut store = SettingsStore::load(data_dir.join("settings.json"));
    let documents_dir = app.path().document_dir().ok();
    if !rl_config::is_valid_install(&store.data.stats_api.install_dir) {
        if let Some(dir) = rl_config::find_install_dirs().into_iter().next() {
            store.update(json!({ "statsApi": { "installDir": dir } }));
        }
    }
    // Détection automatique du pseudo au lancement s'il n'est pas encore renseigné
    if store.data.player.custom_name.trim().is_empty() {
        if let Some(player) = rl_config::detect_player_from_logs(documents_dir.as_deref()) {
            store.update(json!({
                "player": {
                    "platform": player.platform,
                    "customName": player.name
                }
            }));
        }
    }

    let initial_key = sanitize_account_key(&store.data.player.platform, &store.data.player.custom_name);
    let session_file = if !initial_key.is_empty() && initial_key != "default" {
        let p_file = data_dir.join(format!("session_{initial_key}.json"));
        if p_file.exists() {
            p_file
        } else {
            data_dir.join("session.json")
        }
    } else {
        data_dir.join("session.json")
    };

    let saved_mmr = fs::read_to_string(&session_file)
        .ok()
        .and_then(|t| serde_json::from_str::<Session>(&t).ok())
        .map(|s| s.current_mmr)
        .filter(|&m| m > 0.0)
        .unwrap_or(store.data.overlay.starting_mmr);

    let session = if store.data.counting.reset_on_start {
        let mut s = Session::default();
        s.focused_playlist = store.data.counting.focused_playlist.clone();
        s.current_mmr = saved_mmr;
        let (name, div, tier) = calculate_rank(saved_mmr, &s.focused_playlist);
        s.rank_name = name.to_string();
        s.division = div;
        s.rank_tier = tier;
        s
    } else {
        fs::read_to_string(&session_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Session>(&t).ok())
            .unwrap_or_else(|| {
                let mut s = Session::default();
                s.focused_playlist = store.data.counting.focused_playlist.clone();
                s.current_mmr = saved_mmr;
                let (name, div, tier) = calculate_rank(saved_mmr, &s.focused_playlist);
                s.rank_name = name.to_string();
                s.division = div;
                s.rank_tier = tier;
                s
            })
    };

    let rl_port = Arc::new(AtomicU16::new(store.data.stats_api.port));
    let server_port = store.data.server.port;
    let lobby_engine = lobby::create_engine(app.handle(), store.data.lobbytracker.clone(), &data_dir.join("lobby_cache.db"))?;
    let history_file = data_dir.join("history.json");
    let core = Core {
        store,
        tracker: Tracker::new(session),
        connected: false,
        overlay_size: (380.0, 320.0),
        obs_dir: data_dir.join("obs"),
        data_dir: data_dir.clone(),
        session_file,
        history_file,
        log_file,
        documents_dir,
        current_player_key: initial_key,
    };
    core.persist_session();
    let core = Arc::new(Mutex::new(core));

    let web = {
        let core = core.clone();
        let engine = lobby_engine.clone();
        WebServer::start(
            server_port,
            Arc::new(move || core.lock().unwrap().overlay_state()),
            Arc::new(move || lobby::state_value(&engine)),
        )
    };

    app.manage(Ctx {
        core: core.clone(),
        rl_port: rl_port.clone(),
        web,
        toggle_item: Mutex::new(None),
        preview_overlay: Arc::new(AtomicBool::new(false)),
        overlay_visible: Arc::new(AtomicBool::new(false)),
    });
    lobby::start(&handle, lobby_engine);

    // --- Icône près de l'horloge ---
    let enabled = core.lock().unwrap().store.data.overlay.enabled;
    let open = MenuItem::with_id(app, "open", "Ouvrir les paramètres", true, None::<&str>)?;
    let toggle = CheckMenuItem::with_id(app, "toggle", "Afficher l'overlay en jeu (Ctrl+Maj+O)", true, enabled, None::<&str>)?;
    let lobby_toggle = MenuItem::with_id(app, "lobby", "Afficher / masquer l'overlay LobbyTracker (Ctrl+Maj+L)", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", "Réinitialiser la session", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &toggle, &lobby_toggle, &reset, &separator, &quit])?;
    *app.state::<Ctx>().toggle_item.lock().unwrap() = Some(toggle);

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("RL Tracker Suite")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => open_settings(app),
            "toggle" => toggle_overlay(app),
            "lobby" => lobby::toggle_overlay(app),
            "reset" => reset_session(app),
            "quit" => {
                handle_exit_cleanup(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    // --- Fenêtre overlay : transparente, toujours au premier plan, les clics la traversent ---
    let overlay = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("RL Tracker Suite - Overlay")
        .inner_size(380.0, 320.0)
        .position(OFFSCREEN, OFFSCREEN)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .focused(false)
        .visible(true)
        .build()?;
    overlay.set_ignore_cursor_events(true)?;

    // --- Fenêtre overlay du lobby (LobbyTracker) : même principe, page différente ---
    let lobby_overlay = WebviewWindowBuilder::new(app, lobby::OVERLAY, WebviewUrl::App("lobby-overlay.html".into()))
        .title("RL Tracker Suite - Overlay du lobby")
        .inner_size(380.0, 200.0)
        .position(OFFSCREEN, OFFSCREEN)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .focused(false)
        .visible(true)
        .build()?;
    lobby_overlay.set_ignore_cursor_events(true)?;

    // --- Raccourci global Ctrl+Maj+O ---
    {
        use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
        let mods = Some(Modifiers::CONTROL | Modifiers::SHIFT);
        let session_shortcut = Shortcut::new(mods, Code::KeyO);
        let lobby_shortcut = Shortcut::new(mods, Code::KeyL);
        app.handle().plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, pressed, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if pressed == &session_shortcut {
                        toggle_overlay(app);
                    } else if pressed == &lobby_shortcut {
                        lobby::toggle_overlay(app);
                    }
                })
                .build(),
        )?;
        for (shortcut, label) in [(session_shortcut, "Ctrl+Maj+O"), (lobby_shortcut, "Ctrl+Maj+L")] {
            if app.global_shortcut().register(shortcut).is_err() {
                core.lock().unwrap().log(&format!("Raccourci {label} indisponible (déjà utilisé par une autre appli)"));
            }
        }
    }

    // --- Connexion à la Stats API ---
    {
        let app = handle.clone();
        stats_client::spawn(rl_port, move |event| {
            if let ClientEvent::Message(name, data) = &event {
                lobby::on_stats_event(&app, name, data);
            }
            let effects = {
                let ctx = app.state::<Ctx>();
                let mut c = ctx.core.lock().unwrap();
                match event {
                    ClientEvent::Connected => {
                        c.connected = true;
                        c.log("Connecté à Rocket League");
                        Effects { status_changed: true, ..Default::default() }
                    }
                    ClientEvent::Disconnected => {
                        c.connected = false;
                        let rules = c.rules();
                        let mut e = c.tracker.connection_lost(&rules);
                        e.status_changed = true;
                        e
                    }
                    ClientEvent::Message(name, data) => {
                        let rules = c.rules();
                        let identity = c.identity();
                        c.tracker.handle(&name, &data, identity.as_deref(), &rules)
                    }
                }
            };
            after_effects(&app, effects);
        });
    }

    // Game Watcher : veille automatique quand le jeu est fermé ou en arrière-plan (Alt-Tab)
    {
        let app = handle.clone();
        thread::spawn(move || {
            let mut was_running = false;
            let mut run_check_counter: u32 = 0;
            let mut is_running = false;

            loop {
                // Vérifier si le processus Rocket League tourne toutes les ~1.5 secondes
                if run_check_counter % 6 == 0 {
                    let running_now = rl_config::is_rocket_league_running();
                    if running_now && !was_running {
                        was_running = true;
                        let h = app.clone();
                        tauri::async_runtime::spawn(async move {
                            lobby::on_game_launched(&h);
                            // Détection automatique de la plateforme d'où Rocket League a été lancé (style BakkesMod)
                            let mut detected_platform = None;
                            if let Some(info) = rl_config::get_running_rocket_league_info() {
                                let plat_str = info.platform.clone();
                                let target_dir = info.install_dir.clone();
                                {
                                    let ctx = h.state::<Ctx>();
                                    let mut c = ctx.core.lock().unwrap();
                                    if let Some(ref dir) = target_dir {
                                        c.store.update(json!({ "statsApi": { "installDir": dir } }));
                                    }
                                    if plat_str == "epic" || plat_str == "steam" {
                                        c.store.update(json!({ "player": { "platform": plat_str } }));
                                        detected_platform = Some(plat_str.clone());
                                    }
                                    let display_name = if plat_str == "steam" { "Steam" } else if plat_str == "epic" { "Epic Games" } else { "Processus" };
                                    c.log(&format!("Rocket League d\u{e9}tect\u{e9} via {} : {}", display_name, target_dir.as_deref().unwrap_or(&info.exe_path)));
                                }
                                publish(&h);
                            } else {
                                let ctx = h.state::<Ctx>();
                                ctx.core.lock().unwrap().log("Rocket League d\u{e9}tect\u{e9}");
                            }

                            // Tentatives répétées de détection du joueur pendant le démarrage du jeu
                            // Rocket League écrit Launch.log progressivement (command line au début, EOS/PsyNet après ~10s).
                            let h_inner = h.clone();
                            tauri::async_runtime::spawn(async move {
                                for _attempt in 0..10 {
                                    std::thread::sleep(Duration::from_millis(1500));
                                    let docs = h_inner.state::<Ctx>().core.lock().unwrap().documents_dir.clone();
                                    if let Some(player) = rl_config::detect_player_from_logs(docs.as_deref()) {
                                        let final_plat = detected_platform.clone().unwrap_or(player.platform);
                                        let switched = {
                                            let ctx = h_inner.state::<Ctx>();
                                            let mut c = ctx.core.lock().unwrap();
                                            let cur_user = c.store.data.player.custom_name.clone();
                                            let s = c.switch_account_if_needed(&final_plat, &player.name, &player.id);
                                            if s || cur_user != player.name {
                                                c.log(&format!("Profil joueur actualisé : {} ({})", player.name, final_plat));
                                            }
                                            s
                                        };
                                        lobby::on_local_player(&h_inner, &final_plat, &player.id, &player.name);
                                        publish(&h_inner);
                                        let auto_sync = h_inner.state::<Ctx>().core.lock().unwrap().store.data.player.auto_sync_mmr;
                                        if auto_sync {
                                            std::thread::sleep(Duration::from_millis(1000));
                                            let _ = perform_tracker_sync(h_inner.clone()).await;
                                        }
                                        let _ = switched;
                                        break;
                                    }
                                }
                            });
                        });
                    } else if !running_now && was_running {
                        was_running = false;
                        {
                            let ctx = app.state::<Ctx>();
                            ctx.core.lock().unwrap().log("Rocket League fermé — mise en veille de l'overlay");
                        }
                    }
                    is_running = running_now;
                }
                run_check_counter = run_check_counter.wrapping_add(1);

                let (enabled, only_in_game, is_preview) = {
                    let ctx = app.state::<Ctx>();
                    let c = ctx.core.lock().unwrap();
                    (c.store.data.overlay.enabled, c.store.data.overlay.only_in_game, ctx.preview_overlay.load(Ordering::Relaxed))
                };

                let in_game = if only_in_game { rl_config::is_rocket_league_focused() } else { is_running };
                let should_be_visible = if is_preview {
                    true
                } else if !enabled {
                    false
                } else {
                    in_game
                };
                lobby::tick(&app, in_game);

                let is_currently_visible = app.state::<Ctx>().overlay_visible.load(Ordering::Relaxed);
                if should_be_visible {
                    if !is_currently_visible {
                        render_overlay_on_screen(&app);
                    } else if let Some(win) = app.get_webview_window("overlay") {
                        let _ = win.set_always_on_top(true);
                    }
                } else {
                    if is_currently_visible {
                        hide_overlay(&app);
                    }
                }

                thread::sleep(Duration::from_millis(250));
            }
        });
    }

    // Synchronisation automatique du MMR au lancement si un joueur est déjà configuré
    {
        let app = handle.clone();
        tauri::async_runtime::spawn(async move {
            thread::sleep(Duration::from_secs(2));
            let has_user = {
                let ctx = app.state::<Ctx>();
                let c = ctx.core.lock().unwrap();
                let p = &c.store.data.player;
                !p.custom_name.trim().is_empty() || !p.primary_id.trim().is_empty() || !p.detected_name.trim().is_empty()
            };
            if has_user {
                let _ = perform_tracker_sync(app).await;
            }
        });
    }

    if !std::env::args().any(|a| a == "--minimized") {
        if let Some(settings_win) = app.get_webview_window("settings") {
            let _ = settings_win.show();
            let _ = settings_win.set_focus();
        }
    }

    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_settings(app)))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_settings,
            reset_session_cmd,
            set_mmr_cmd,
            statsapi_status,
            statsapi_detect,
            statsapi_select_platform,
            detect_player_now,
            statsapi_choose,
            statsapi_enable,
            open_obs_folder,
            open_browser_url,
            sync_tracker_mmr,
            delete_history_session,
            get_autostart,
            set_autostart,
            quit_app,
            overlay_state,
            overlay_size,
            drag_overlay_by,
            save_overlay_drag_finish,
            get_overlay_preview,
            set_overlay_preview,
            app_version,
            set_overlay_custom_position,
            lobby::lt_state,
            lobby::lt_save_config,
            lobby::lt_set_preview,
            lobby::lt_overlay_size,
            lobby::lt_drag_by,
            lobby::lt_drag_finish,
        ])
        .on_window_event(|window, event| {
            // Fermer la fenêtre des paramètres la cache seulement : l'overlay reste actif.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    api.prevent_close();
                    let _ = window.hide();
                    let ctx = window.app_handle().state::<Ctx>();
                    if ctx.preview_overlay.swap(false, Ordering::Relaxed) {
                        apply_overlay(window.app_handle());
                    }
                }
            }
        })
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("Impossible de démarrer RL Tracker Suite");
}
