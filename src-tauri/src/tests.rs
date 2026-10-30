//! Tests du moteur : fichiers .ini, logique de session via un faux Rocket League, serveur OBS.

use crate::rl_config;
use crate::session::{CountRules, Effects, Session, Tracker};
use crate::settings::SettingsStore;
use crate::stats_client::{self, ClientEvent, JsonStreamParser};
use crate::web_server::WebServer;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicU16;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

fn rules() -> CountRules {
    CountRules { online_only: true, mode: "all".into(), playlists: vec![], count_leave_as_loss: true, mmr_delta: 9.0, focused_playlist: "2v2".into(), auto_switch_playlist: true }
}

fn tick(guid: &str, winner: &str) -> Value {
    json!({
        "MatchGuid": guid,
        "Players": [
            { "Name": "Dirayor", "PrimaryId": "Epic|me|0", "TeamNum": 0, "Goals": 2, "Shots": 3, "Assists": 1, "Saves": 1, "Demos": 1, "Score": 400 },
            { "Name": "Adversaire {\"}", "PrimaryId": "Epic|op|0", "TeamNum": 1, "Goals": 1 }
        ],
        "Game": {
            "PlaylistId": 11, "bHasTarget": true, "bReplay": false, "Target": { "Name": "Dirayor" },
            "bHasWinner": !winner.is_empty(), "Winner": winner,
            "Teams": [ { "Name": "Blue", "TeamNum": 0 }, { "Name": "Orange", "TeamNum": 1 } ]
        }
    })
}

#[test]
fn ini_lecture_ecriture() {
    let text = "[Core.System]\r\nFoo=1\r\n\r\n[TAGame.MatchStatsExporter_TA]\r\nPort=49123\r\nPacketSendRate=0\r\n\r\n[Other]\r\nX=2\r\n";
    let out = rl_config::set_ini_values(text, &[("PacketSendRate", "10".into()), ("Port", "49123".into())]);
    assert_eq!(rl_config::get_ini_value(&out, "PacketSendRate").as_deref(), Some("10"));
    assert!(out.contains("[Other]\r\nX=2"));
    assert!(out.contains("Foo=1"));

    let empty = rl_config::set_ini_values("", &[("PacketSendRate", "10".into())]);
    assert_eq!(empty, "[TAGame.MatchStatsExporter_TA]\nPacketSendRate=10\n");

    let other = rl_config::set_ini_values("[A]\nB=1\n", &[("PacketSendRate", "30".into())]);
    assert_eq!(rl_config::get_ini_value(&other, "PacketSendRate").as_deref(), Some("30"));
    assert_eq!(rl_config::get_ini_value(&other, "B"), None);
}

#[test]
fn activation_stats_api_utf16() {
    let root = std::env::temp_dir().join(format!("rlss-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let config = root.join("rocketleague").join("TAGame").join("Config");
    std::fs::create_dir_all(&config).unwrap();
    let file = config.join("DefaultStatsAPI.ini");
    let mut bytes = vec![0xFF, 0xFE];
    for u in "[TAGame.MatchStatsExporter_TA]\r\nPacketSendRate=0\r\nPort=49123\r\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&file, bytes).unwrap();

    let install = root.join("rocketleague").to_string_lossy().into_owned();
    assert!(rl_config::is_valid_install(&install));
    assert_eq!(rl_config::read_status(&install, None)["enabled"], false);

    rl_config::enable_stats_api(&install, None, 10, 49123).unwrap();
    let status = rl_config::read_status(&install, None);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["files"][0]["packetSendRate"], 10.0);

    let raw = std::fs::read(&file).unwrap();
    assert_eq!(&raw[..2], &[0xFF, 0xFE], "encodage UTF-16 conservé");
    assert!(config.join("DefaultStatsAPI.ini.bak").exists(), "sauvegarde créée");
    assert_eq!(rl_config::normalize_install_dir(&config).unwrap(), root.join("rocketleague"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn decoupage_flux_json() {
    let mut parser = JsonStreamParser::default();
    let mut out = vec![];
    let stream = br#"{"Event":"A","Data":"{\"x\":1}"}{"Event":"B","Data":{"s":"}{"}}"#;
    parser.feed(&stream[..13], &mut out);
    assert!(out.is_empty());
    parser.feed(&stream[13..], &mut out);
    assert_eq!(out.len(), 2);
    let (name, data) = stats_client::decode_message(out[0].clone()).unwrap();
    assert_eq!((name.as_str(), data["x"].as_i64()), ("A", Some(1)));
    assert_eq!(out[1]["Data"]["s"], "}{");
}

/// Scénario complet via un vrai socket TCP, comme le ferait Rocket League.
#[test]
fn session_via_faux_rocket_league() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let tracker = Arc::new(Mutex::new(Tracker::new(Session::default())));
    let detected = Arc::new(Mutex::new(None::<String>));
    let (done_tx, done_rx) = mpsc::channel::<&'static str>();

    {
        let tracker = tracker.clone();
        let detected = detected.clone();
        stats_client::spawn(Arc::new(AtomicU16::new(port)), move |event| {
            let mut t = tracker.lock().unwrap();
            let effects: Effects = match event {
                ClientEvent::Connected => Effects::default(),
                ClientEvent::Disconnected => {
                    let e = t.connection_lost(&rules());
                    let _ = done_tx.send("disconnected");
                    e
                }
                ClientEvent::Message(name, data) => {
                    let id = detected.lock().unwrap().clone();
                    let e = t.handle(&name, &data, id.as_deref(), &rules());
                    if name == "Sync" {
                        let _ = done_tx.send("sync");
                    }
                    e
                }
            };
            if let Some((id, _)) = effects.detected {
                *detected.lock().unwrap() = Some(id);
            }
        });
    }

    let (mut sock, _) = listener.accept().unwrap();
    let mut send = |event: &str, data: Value| {
        let msg = json!({ "Event": event, "Data": data });
        sock.write_all(msg.to_string().as_bytes()).unwrap();
    };
    let sync = |send: &mut dyn FnMut(&str, Value)| {
        send("Sync", json!({}));
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), "sync");
    };

    // Match 1 : victoire, avec une démolition subie ; Data parfois en chaîne JSON.
    send("MatchCreated", json!({ "MatchGuid": "G1" }));
    send("RoundStarted", Value::String(json!({ "MatchGuid": "G1" }).to_string()));
    send("UpdateState", tick("G1", ""));
    send("StatfeedEvent", json!({ "MatchGuid": "G1", "EventName": "Demolish", "SecondaryTarget": { "Name": "Dirayor" } }));
    send("MatchEnded", json!({ "MatchGuid": "G1", "WinnerTeamNum": 0 }));
    send("UpdateState", tick("G1", ""));
    send("MatchDestroyed", json!({ "MatchGuid": "G1" }));
    sync(&mut send);
    {
        let t = tracker.lock().unwrap();
        let s = &t.session;
        assert_eq!((s.games, s.wins, s.losses, s.streak), (1, 1, 0, 1));
        assert_eq!((s.totals.goals, s.totals.deaths), (2, 1));
    }
    assert_eq!(detected.lock().unwrap().as_deref(), Some("Epic|me|0"));

    // Match 2 : victoire -> streak 2
    for e in ["MatchCreated", "RoundStarted"] { send(e, json!({ "MatchGuid": "G2" })); }
    send("UpdateState", tick("G2", ""));
    send("MatchEnded", json!({ "MatchGuid": "G2", "WinnerTeamNum": 0 }));
    send("MatchDestroyed", json!({ "MatchGuid": "G2" }));
    sync(&mut send);
    assert_eq!(tracker.lock().unwrap().session.streak, 2);

    // Match 3 : quitté en cours -> défaite (abandon)
    for e in ["MatchCreated", "RoundStarted"] { send(e, json!({ "MatchGuid": "G3" })); }
    send("UpdateState", tick("G3", ""));
    send("MatchDestroyed", json!({ "MatchGuid": "G3" }));
    sync(&mut send);
    {
        let t = tracker.lock().unwrap();
        let s = &t.session;
        assert_eq!((s.games, s.wins, s.losses, s.streak, s.best_streak), (3, 2, 1, -1, 2));
        assert!(s.recent[2].abandoned);
    }

    // Match 4 : gagnant connu sans MatchEnded (forfait adverse) -> victoire
    for e in ["MatchCreated", "RoundStarted"] { send(e, json!({ "MatchGuid": "G4" })); }
    send("UpdateState", tick("G4", "Blue"));
    send("MatchDestroyed", json!({ "MatchGuid": "G4" }));
    sync(&mut send);
    assert_eq!(tracker.lock().unwrap().session.streak, 1);

    // Match 5 : hors ligne (sans MatchGuid) -> ignoré
    for e in ["MatchCreated", "RoundStarted"] { send(e, json!({})); }
    send("UpdateState", tick("", ""));
    send("MatchEnded", json!({ "WinnerTeamNum": 1 }));
    send("MatchDestroyed", json!({}));
    sync(&mut send);
    assert_eq!(tracker.lock().unwrap().session.games, 4);

    // Match 6 : coupure du socket en plein match -> abandon
    for e in ["MatchCreated", "RoundStarted"] { send(e, json!({ "MatchGuid": "G6" })); }
    send("UpdateState", tick("G6", ""));
    sync(&mut send);
    drop(send);
    drop(sock);
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), "disconnected");
    {
        let t = tracker.lock().unwrap();
        let s = &t.session;
        assert_eq!((s.games, s.streak), (5, -1));
        let pl = &s.playlists["11"];
        assert_eq!((pl.name.as_str(), pl.wins, pl.losses), ("Ranked 2v2", 3, 2));
        // Sérialisation attendue par l'interface (camelCase)
        let v = serde_json::to_value(s).unwrap();
        assert!(v.get("bestStreak").is_some() && v.get("startedAt").is_some());
    }

    // Reconnexion automatique
    listener.set_nonblocking(false).unwrap();
    let (_again, _) = listener.accept().unwrap();
}

#[test]
fn fin_de_match_declenche_une_seule_synchronisation() {
    let mut t = Tracker::new(Session::default());
    let r = rules();
    for e in ["MatchCreated", "RoundStarted"] {
        t.handle(e, &json!({ "MatchGuid": "F1" }), None, &r);
    }
    let update = t.handle("UpdateState", &tick("F1", ""), None, &r);
    assert!(!update.match_finished, "rien à synchroniser en plein match");
    let end = t.handle("MatchEnded", &json!({ "MatchGuid": "F1", "WinnerTeamNum": 0 }), None, &r);
    assert!(end.match_finished && end.session_changed);
    let closed = t.handle("MatchDestroyed", &json!({ "MatchGuid": "F1" }), None, &r);
    assert!(!closed.match_finished, "pas de seconde synchronisation à la fermeture du match");
}

#[test]
fn filtre_ranked() {
    let mut t = Tracker::new(Session::default());
    let r = CountRules { mode: "ranked".into(), ..rules() };
    let mut casual = tick("C1", "");
    casual["Game"]["PlaylistId"] = json!(2);
    for e in ["MatchCreated", "RoundStarted"] { t.handle(e, &json!({ "MatchGuid": "C1" }), None, &r); }
    t.handle("UpdateState", &casual, None, &r);
    t.handle("MatchEnded", &json!({ "MatchGuid": "C1", "WinnerTeamNum": 0 }), None, &r);
    assert_eq!(t.session.games, 0);
}

#[test]
fn parametres_fusion_et_valeurs_invalides() {
    let path = std::env::temp_dir().join(format!("rlss-settings-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = SettingsStore::load(path.clone());
    assert_eq!(store.data.server.port, 5177);
    assert!(store.update(json!({ "overlay": { "scale": 1.25, "fields": ["streak"] }, "statsApi": { "packetSendRate": 30 } })));
    assert!(!store.update(json!({ "server": { "port": "pas un nombre" } })), "valeur invalide refusée");
    let reloaded = SettingsStore::load(path.clone());
    assert_eq!(reloaded.data.overlay.scale, 1.25);
    assert_eq!(reloaded.data.overlay.fields, vec!["streak"]);
    assert_eq!(reloaded.data.overlay.position, "top-left", "le reste est conservé");
    assert_eq!(reloaded.data.stats_api.packet_send_rate, 30);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reglages_lobbytracker_dans_le_meme_fichier() {
    let path = std::env::temp_dir().join(format!("rlss-lt-settings-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = SettingsStore::load(path.clone());
    assert_eq!(store.data.lobbytracker.overlay_position, "top-right");
    assert!(store.update(json!({ "lobbytracker": { "overlay_scale": 150, "filters": { "on_tilt": false } } })));
    assert!(store.update(json!({ "overlay": { "scale": 1.5 } })), "les réglages de session restent indépendants");
    let reloaded = SettingsStore::load(path.clone());
    assert_eq!(reloaded.data.lobbytracker.overlay_scale, 150);
    assert!(!reloaded.data.lobbytracker.filters.on_tilt);
    assert!(reloaded.data.lobbytracker.filters.smurf_suspect, "le reste des filtres est conservé");
    assert_eq!(reloaded.data.overlay.scale, 1.5);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn placement_des_overlays_partage_entre_session_et_lobby() {
    use crate::overlay_origin;
    let (mon, size, win) = ((0.0, 0.0), (1920.0, 1080.0), (400.0, 200.0));
    let at = |p: &str| overlay_origin(mon, size, win, p, (24.0, 24.0), (None, None));
    assert_eq!(at("top-left"), (24.0, 24.0));
    assert_eq!(at("top-center"), (784.0, 24.0), "le centrage ajoute la marge X, comme l overlay de session");
    assert_eq!(at("top-right"), (1496.0, 24.0));
    assert_eq!(at("bottom-left"), (24.0, 856.0));
    assert_eq!(at("bottom-right"), (1496.0, 856.0));
    assert_eq!(overlay_origin(mon, size, win, "custom", (24.0, 24.0), (Some(300.0), Some(120.0))), (300.0, 120.0));
    // Deuxième écran positionné à droite du premier.
    assert_eq!(overlay_origin((1920.0, 0.0), (1280.0, 720.0), win, "top-left", (24.0, 24.0), (None, None)), (1944.0, 24.0));
}

#[test]
fn reprise_des_donnees_de_l_ancienne_appli() {
    let root = std::env::temp_dir().join(format!("rlts-legacy-{}", std::process::id()));
    let (old, new) = (root.join("old"), root.join("new"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(old.join("settings.json"), r#"{"overlay":{"scale":1.5}}"#).unwrap();
    std::fs::write(old.join("history.json"), "[]").unwrap();
    std::fs::write(old.join("session_epic_neyor.json"), "{}").unwrap();
    std::fs::write(old.join("journal.txt"), "ignoré").unwrap();

    assert_eq!(crate::import_legacy_data(&new, &old), 3);
    assert!(!new.join("journal.txt").exists());
    let imported = SettingsStore::load(new.join("settings.json"));
    assert_eq!(imported.data.overlay.scale, 1.5);
    assert_eq!(imported.data.lobbytracker.overlay_position, "top-right", "section LobbyTracker par défaut");

    // Déjà des réglages ici : on ne touche à rien.
    std::fs::write(old.join("settings.json"), r#"{"overlay":{"scale":0.7}}"#).unwrap();
    assert_eq!(crate::import_legacy_data(&new, &old), 0);
    assert_eq!(SettingsStore::load(new.join("settings.json")).data.overlay.scale, 1.5);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pseudo_encode_pour_une_url() {
    use crate::encode_path_segment as enc;
    assert_eq!(enc("neyor."), "neyor.");
    assert_eq!(enc("Jean Dupont"), "Jean%20Dupont");
    assert_eq!(enc("é"), "%C3%A9");
    assert_eq!(enc("a/b?x=1&y#z%"), "a%2Fb%3Fx%3D1%26y%23z%25");
}

#[test]
fn serveur_obs() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let web = WebServer::start(
        port,
        Arc::new(|| json!({ "session": { "wins": 3 } })),
        Arc::new(|| json!({ "snapshot": { "players": [], "pending": 2 } })),
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(web.error().is_none());

    let get = |path: &str| {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).unwrap();
        s
    };
    let mut page = String::new();
    get("/overlay").read_to_string(&mut page).unwrap();
    assert!(page.starts_with("HTTP/1.1 200") && page.contains("EventSource"));

    let mut events = get("/events");
    events.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut buf = [0u8; 4096];
    let mut got = String::new();
    while !got.contains("\"wins\":3") {
        let n = events.read(&mut buf).unwrap();
        got.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    std::thread::sleep(Duration::from_millis(50));
    web.broadcast(&json!({ "session": { "wins": 4 } }));
    let mut got = String::new();
    while !got.contains("\"wins\":4") {
        let n = events.read(&mut buf).unwrap();
        got.push_str(&String::from_utf8_lossy(&buf[..n]));
    }

    // Overlay du lobby (LobbyTracker) : même serveur, canal séparé.
    let mut lobby_page = String::new();
    get("/lobby").read_to_string(&mut lobby_page).unwrap();
    assert!(lobby_page.starts_with("HTTP/1.1 200") && lobby_page.contains("/lobby/events"));
    let mut lobby_events = get("/lobby/events");
    lobby_events.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut got = String::new();
    while !got.contains("\"pending\":2") {
        let n = lobby_events.read(&mut buf).unwrap();
        got.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    std::thread::sleep(Duration::from_millis(50));
    web.broadcast_lobby(&json!({ "snapshot": { "pending": 0 } }));
    let mut got = String::new();
    while !got.contains("\"pending\":0") {
        let n = lobby_events.read(&mut buf).unwrap();
        got.push_str(&String::from_utf8_lossy(&buf[..n]));
    }

    let taken = WebServer::start(port, Arc::new(|| json!({})), Arc::new(|| json!({})));
    assert!(taken.error().unwrap().contains("déjà utilisé"));
}

#[test]
fn mmr_et_calcul_rang() {
    use crate::session::calculate_rank;
    let (name, div, tier) = calculate_rank(0.0, "2v2");
    assert_eq!((name, div, tier), ("Unranked", 1, 0));

    // 1656 ELO en 2v2 correspond exactement à Grand Champion II Division 3
    let (name, div, tier) = calculate_rank(1656.0, "2v2");
    assert_eq!((name, div, tier), ("Grand Champion II", 3, 20));

    // Paliers 2v2
    let (name, div, tier) = calculate_rank(855.0, "2v2");
    assert_eq!((name, div, tier), ("Diamond I", 2, 13));

    let (name, div, tier) = calculate_rank(1500.0, "2v2");
    assert_eq!((name, div, tier), ("Grand Champion I", 2, 19));

    let (name, div, tier) = calculate_rank(1950.0, "2v2");
    assert_eq!((name, div, tier), ("Supersonic Legend", 1, 22));

    // Paliers 1v1 spécifiques
    let (name, div, tier) = calculate_rank(1200.0, "1v1");
    assert_eq!((name, div, tier), ("Grand Champion I", 2, 19));

    let mut tracker = Tracker::new(Session::default());
    tracker.set_mmr(1656.0, "2v2");
    assert_eq!(tracker.session.current_mmr, 1656.0);
    assert_eq!(tracker.session.rank_name, "Grand Champion II");
    assert_eq!(tracker.session.division, 3);
    assert_eq!(tracker.session.rank_tier, 20);
}

#[test]
fn parse_player_from_log_epic() {
    let log_sample = r#"
[0001.23] Init: Command line: -epicapp=Sugar -epicenv=Prod -epicusername="neyor." -epicuserid=6897812333f244858dddff1fbb06554f
[0004.56] Log: HandleLocalPlayerLoginStatusChanged PlayerName=neyor. PlayerID=Epic|6897812333f244858dddff1fbb06554f|0 bLoggedIn=1
"#;
    let detected = rl_config::parse_player_from_log_content(log_sample).expect("Joueur Epic doit être détecté");
    assert_eq!(detected.platform, "epic");
    assert_eq!(detected.name, "neyor.");
    assert_eq!(detected.id, "6897812333f244858dddff1fbb06554f");
}

#[test]
fn parse_player_from_log_steam() {
    let log_sample = r#"
[0001.23] Init: Steam initialized
[0005.12] Log: HandleLocalPlayerLoginStatusChanged PlayerName=SquishyMuffinz PlayerID=Steam|76561198000000000|0 bLoggedIn=1
"#;
    let detected = rl_config::parse_player_from_log_content(log_sample).expect("Joueur Steam doit être détecté");
    assert_eq!(detected.platform, "steam");
    assert_eq!(detected.name, "SquishyMuffinz");
    assert_eq!(detected.id, "76561198000000000");

    let log_sample_spaces = r#"
[0001.23] Init: Steam initialized
[0005.12] Log: HandleLocalPlayerLoginStatusChanged PlayerName=Epsilon Drxzm PlayerID=Steam|76561198012345678|0 LoginStatus=LS_LoggedIn IsPrimary=True IsInParty=False
"#;
    let detected_spaces = rl_config::parse_player_from_log_content(log_sample_spaces).expect("Joueur Steam avec espaces doit être détecté");
    assert_eq!(detected_spaces.platform, "steam");
    assert_eq!(detected_spaces.name, "Epsilon Drxzm");
    assert_eq!(detected_spaces.id, "76561198012345678");
}

#[test]
fn detect_installations_structure() {
    let installs = rl_config::detect_game_installations();
    // Doit renvoyer une structure DetectedInstalls valide sans paniquer
    let _ = installs.epic;
    let _ = installs.steam;
    let _info = rl_config::get_running_rocket_league_info();
}

#[test]
fn auto_switch_playlist_on_match_tick() {
    let mut tracker = Tracker::new(Session::default());
    assert_eq!(tracker.session.focused_playlist, "2v2");

    // Simulation d'un match de 1v1 (PlaylistId = 10)
    let d = json!({
        "MatchGuid": "test-1v1",
        "Players": [{ "Name": "Me", "PrimaryId": "Epic|me|0", "TeamNum": 0 }],
        "Game": { "PlaylistId": 10, "Target": { "Name": "Me" } }
    });
    let r = rules();
    let effects = tracker.handle("UpdateState", &d, Some("NAME:epic|Me"), &r);
    assert_eq!(tracker.session.focused_playlist, "1v1");
    assert!(effects.session_changed);
    assert!(!effects.match_finished, "changer de mode au début d'un match ne déclenche pas de synchronisation");

    // Simulation d'un match de 3v3 (PlaylistId = 13)
    let d3 = json!({
        "MatchGuid": "test-3v3",
        "Players": [{ "Name": "Me", "PrimaryId": "Epic|me|0", "TeamNum": 0 }],
        "Game": { "PlaylistId": 13, "Target": { "Name": "Me" } }
    });
    let effects3 = tracker.handle("UpdateState", &d3, Some("NAME:epic|Me"), &r);
    assert_eq!(tracker.session.focused_playlist, "3v3");
    assert!(effects3.session_changed);
}

#[test]
fn sanitize_account_keys() {
    assert_eq!(crate::sanitize_account_key("Epic", "Neyor DEV"), "epic_Neyor_DEV");
    assert_eq!(crate::sanitize_account_key("Steam", "Player_123"), "steam_Player_123");
    assert_eq!(crate::sanitize_account_key("", ""), "default");
}

#[test]
fn test_live_player_detection() {
    let paths = rl_config::find_launch_log_paths(None);
    for p in &paths {
        let meta = std::fs::metadata(p).and_then(|m| m.modified());
        println!("Candidate log path: {:?}, mod: {:?}", p, meta);
    }
    let detected = rl_config::detect_player_from_logs(None);
    println!("Detected player: {:?}", detected);
}
