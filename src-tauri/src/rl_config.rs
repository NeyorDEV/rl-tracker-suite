//! Trouve le dossier d'installation de Rocket League et règle la Stats API dans ses fichiers .ini.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedPlayer {
    pub platform: String,
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DetectedInstalls {
    pub epic: Option<String>,
    pub steam: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunningGameInfo {
    pub pid: u32,
    pub exe_path: String,
    pub install_dir: Option<String>,
    pub platform: String,
}

const SECTION: &str = "[TAGame.MatchStatsExporter_TA]";

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

pub fn is_valid_install(dir: &str) -> bool {
    !dir.is_empty() && Path::new(dir).join("TAGame").join("Config").is_dir()
}

/// Accepte aussi un sous-dossier (TAGame, Config, Binaries...) choisi par erreur.
pub fn normalize_install_dir(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    for _ in 0..4 {
        let c = current?;
        if c.join("TAGame").join("Config").is_dir() {
            return Some(c.to_path_buf());
        }
        current = c.parent();
    }
    None
}

fn epic_install_dirs() -> Vec<String> {
    let manifests = Path::new(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = fs::read_dir(manifests) else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.path().extension().map_or(false, |x| x == "item"))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|item| {
            let name = format!(
                "{} {}",
                item["DisplayName"].as_str().unwrap_or(""),
                item["AppName"].as_str().unwrap_or("")
            )
            .to_lowercase();
            name.contains("rocket league") || item["AppName"] == "Sugar"
        })
        .filter_map(|item| item["InstallLocation"].as_str().map(String::from))
        .collect()
}

fn steam_roots() -> Vec<String> {
    let mut roots = vec![r"C:\Program Files (x86)\Steam".to_string(), r"C:\Program Files\Steam".to_string()];
    let mut cmd = Command::new("reg");
    cmd.args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"]);
    hide_console(&mut cmd);
    if let Ok(out) = cmd.output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some((_, value)) = line.split_once("REG_SZ") {
                roots.insert(0, value.trim().replace('/', "\\"));
            }
        }
    }
    roots
}

fn steam_install_dirs() -> Vec<String> {
    let mut dirs = vec![];
    for root in steam_roots() {
        let mut libraries = vec![root.clone()];
        if let Ok(vdf) = fs::read_to_string(Path::new(&root).join("steamapps").join("libraryfolders.vdf")) {
            for line in vdf.lines() {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 4 && parts[1] == "path" {
                    libraries.push(parts[3].replace(r"\\", r"\"));
                }
            }
        }
        for lib in libraries {
            dirs.push(Path::new(&lib).join("steamapps").join("common").join("rocketleague").to_string_lossy().into_owned());
        }
    }
    dirs
}

pub fn detect_game_installations() -> DetectedInstalls {
    let mut epic_candidates = epic_install_dirs();
    epic_candidates.push(r"C:\Program Files\Epic Games\rocketleague".into());
    epic_candidates.push(r"D:\Epic Games\rocketleague".into());
    let mut seen_epic = std::collections::HashSet::new();
    let epic = epic_candidates
        .into_iter()
        .filter(|d| seen_epic.insert(d.to_lowercase()))
        .find(|d| is_valid_install(d));

    let mut steam_candidates = steam_install_dirs();
    steam_candidates.push(r"C:\Program Files (x86)\Steam\steamapps\common\rocketleague".into());
    steam_candidates.push(r"C:\Program Files\Steam\steamapps\common\rocketleague".into());
    let mut seen_steam = std::collections::HashSet::new();
    let steam = steam_candidates
        .into_iter()
        .filter(|d| seen_steam.insert(d.to_lowercase()))
        .find(|d| is_valid_install(d));

    DetectedInstalls { epic, steam }
}

pub fn find_install_dirs() -> Vec<String> {
    let installs = detect_game_installations();
    let mut list = Vec::new();
    if let Some(e) = installs.epic {
        list.push(e);
    }
    if let Some(s) = installs.steam {
        list.push(s);
    }
    list
}

pub fn find_launch_log_paths(documents_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(doc) = documents_dir {
        candidates.push(doc.join("My Games").join("Rocket League").join("TAGame").join("Logs").join("Launch.log"));
    }
    #[cfg(windows)]
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let p = Path::new(&user_profile);
        candidates.push(p.join("OneDrive").join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Logs").join("Launch.log"));
        candidates.push(p.join("Documents").join("My Games").join("Rocket League").join("TAGame").join("Logs").join("Launch.log"));
    }
    for d in find_install_dirs() {
        candidates.push(Path::new(&d).join("TAGame").join("Logs").join("Launch.log"));
    }

    let mut seen = std::collections::HashSet::new();
    let mut existing: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|p| p.exists())
        .filter(|p| seen.insert(p.to_string_lossy().to_lowercase()))
        .collect();

    existing.sort_by(|a, b| {
        let t_a = fs::metadata(a).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let t_b = fs::metadata(b).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        t_b.cmp(&t_a)
    });

    // Élimine les fichiers fantômes / obsolètes (> 48 heures) si un fichier plus récent existe
    let now = std::time::SystemTime::now();
    let max_stale = std::time::Duration::from_secs(48 * 3600);
    let fresh: Vec<PathBuf> = existing
        .iter()
        .filter(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .map(|t| now.duration_since(t).map(|d| d < max_stale).unwrap_or(true))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    if !fresh.is_empty() {
        // Si Rocket League est en cours d'exécution, seul le fichier actif le plus récent compte
        if is_rocket_league_running() {
            vec![fresh[0].clone()]
        } else {
            fresh
        }
    } else {
        existing.into_iter().take(1).collect()
    }
}

pub fn read_file_shared(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        let mut opts = fs::OpenOptions::new();
        opts.read(true);
        opts.share_mode(7); // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        if let Ok(mut f) = opts.open(path) {
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                return Some(String::from_utf8_lossy(&buf).into_owned());
            }
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        if let Ok(mut f) = fs::File::open(path) {
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                return Some(String::from_utf8_lossy(&buf).into_owned());
            }
        }
    }
    None
}

pub fn parse_player_from_log_content(content: &str) -> Option<DetectedPlayer> {
    let mut detected_cmd_user: Option<String> = None;
    let mut detected_cmd_id: Option<String> = None;

    // Scan backwards to find the most recent session's login line first
    for line in content.lines().rev() {
        if line.contains("HandleLocalPlayerLoginStatusChanged") {
            if let Some(name_pos) = line.find("PlayerName=") {
                let rest = &line[name_pos + "PlayerName=".len()..];
                let name = rest.split_whitespace().next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
                if let Some(id_pos) = line.find("PlayerID=") {
                    let id_rest = &line[id_pos + "PlayerID=".len()..];
                    let id_token = id_rest.split_whitespace().next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
                    let parts: Vec<&str> = id_token.split('|').collect();
                    let raw_platform = parts.get(0).unwrap_or(&"epic").to_lowercase();
                    let platform = if raw_platform.contains("steam") { "steam" } else { "epic" }.to_string();
                    let raw_id = parts.get(1).unwrap_or(&"").to_string();
                    if !name.is_empty() {
                        return Some(DetectedPlayer {
                            platform,
                            name: name.to_string(),
                            id: if raw_id.is_empty() { id_token.to_string() } else { raw_id },
                        });
                    }
                }
            }
        }

        if detected_cmd_user.is_none() && line.contains("-epicusername=") {
            if let Some(pos) = line.find("-epicusername=") {
                let rest = &line[pos + "-epicusername=".len()..];
                let user = rest.split_whitespace().next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
                if !user.is_empty() {
                    detected_cmd_user = Some(user.to_string());
                }
            }
        }
        if detected_cmd_id.is_none() && line.contains("-epicuserid=") {
            if let Some(pos) = line.find("-epicuserid=") {
                let rest = &line[pos + "-epicuserid=".len()..];
                let id = rest.split_whitespace().next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
                if !id.is_empty() {
                    detected_cmd_id = Some(id.to_string());
                }
            }
        }
    }

    if let Some(user) = detected_cmd_user {
        return Some(DetectedPlayer {
            platform: "epic".to_string(),
            name: user,
            id: detected_cmd_id.unwrap_or_default(),
        });
    }

    None
}

pub fn detect_player_from_logs(documents_dir: Option<&Path>) -> Option<DetectedPlayer> {
    let paths = find_launch_log_paths(documents_dir);
    for path in paths {
        if let Some(content) = read_file_shared(&path) {
            if let Some(player) = parse_player_from_log_content(&content) {
                return Some(player);
            }
        }
    }
    None
}

/// Fichiers à modifier : TAStatsAPI.ini / DefaultStatsAPI.ini (dossier du jeu)
/// et TAStatsAPI.ini (Documents & OneDrive).
fn target_files(install_dir: &str, documents_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut files = vec![];
    let config = Path::new(install_dir).join("TAGame").join("Config");
    files.push(config.join("DefaultStatsAPI.ini"));
    files.push(config.join("TAStatsAPI.ini"));

    if let Some(docs) = documents_dir {
        files.push(docs.join("My Games").join("Rocket League").join("TAGame").join("Config").join("TAStatsAPI.ini"));
    }
    #[cfg(windows)]
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let docs = PathBuf::from(&user_profile).join("Documents");
        files.push(docs.join("My Games").join("Rocket League").join("TAGame").join("Config").join("TAStatsAPI.ini"));
        let one_drive = PathBuf::from(&user_profile).join("OneDrive").join("Documents");
        files.push(one_drive.join("My Games").join("Rocket League").join("TAGame").join("Config").join("TAStatsAPI.ini"));
    }

    let mut seen = std::collections::HashSet::new();
    files.into_iter().filter(|f| seen.insert(f.to_string_lossy().to_lowercase())).collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Bom,
}

fn read_ini(path: &Path) -> (String, Encoding) {
    let Ok(bytes) = fs::read(path) else { return (String::new(), Encoding::Utf8) };
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        (String::from_utf16_lossy(&units), Encoding::Utf16Bom)
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(&bytes[3..]).into_owned(), Encoding::Utf8Bom)
    } else {
        (String::from_utf8_lossy(&bytes).into_owned(), Encoding::Utf8)
    }
}

fn write_ini(path: &Path, text: &str, encoding: Encoding) -> io::Result<()> {
    let bytes = match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Bom => {
            let mut out = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, bytes)
}

fn is_section_header(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('[') && t.ends_with(']')
}

fn section_range(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim().eq_ignore_ascii_case(SECTION))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| is_section_header(l))
        .map_or(lines.len(), |(i, _)| i);
    Some((start, end))
}

/// Si `line` est de la forme `key = valeur` (insensible à la casse), renvoie la valeur.
fn key_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once('=')?;
    k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
}

pub fn get_ini_value(text: &str, key: &str) -> Option<String> {
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let (start, end) = section_range(&lines)?;
    lines[start + 1..end].iter().find_map(|l| key_value(l, key).map(String::from))
}

pub fn set_ini_values(text: &str, values: &[(&str, String)]) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let (start, mut end) = match section_range(&lines) {
        Some(range) => range,
        None => {
            while lines.last().map_or(false, |l| l.trim().is_empty()) {
                lines.pop();
            }
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(SECTION.to_string());
            (lines.len() - 1, lines.len())
        }
    };
    for (key, value) in values {
        let line = format!("{key}={value}");
        if let Some(i) = (start + 1..end).find(|&i| key_value(&lines[i], key).is_some()) {
            lines[i] = line;
        } else {
            let mut insert_at = end;
            while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            lines.insert(insert_at, line);
            end += 1;
        }
    }
    let mut out = lines.join(eol);
    out.push_str(eol);
    out
}

pub fn read_status(install_dir: &str, documents_dir: Option<&Path>) -> Value {
    let installs = detect_game_installations();
    let running_info = get_running_rocket_league_info();
    let norm_cur = install_dir.trim().to_lowercase();
    let epic_norm = installs.epic.as_ref().map(|s| s.trim().to_lowercase());
    let steam_norm = installs.steam.as_ref().map(|s| s.trim().to_lowercase());
    let active_platform = if !norm_cur.is_empty() && epic_norm.as_ref() == Some(&norm_cur) {
        "epic"
    } else if !norm_cur.is_empty() && steam_norm.as_ref() == Some(&norm_cur) {
        "steam"
    } else if !norm_cur.is_empty() {
        "custom"
    } else {
        "none"
    };

    if !is_valid_install(install_dir) {
        return json!({
            "installDir": install_dir,
            "valid": false,
            "enabled": false,
            "files": [],
            "installs": installs,
            "activePlatform": active_platform,
            "runningGame": running_info,
        });
    }
    let files: Vec<Value> = target_files(install_dir, documents_dir)
        .iter()
        .map(|path| {
            let (text, _) = read_ini(path);
            let rate = get_ini_value(&text, "PacketSendRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let port = get_ini_value(&text, "Port").and_then(|v| v.parse::<u16>().ok()).unwrap_or(49123);
            json!({ "path": path, "exists": path.exists(), "packetSendRate": rate, "port": port })
        })
        .collect();
    let existing: Vec<&Value> = files.iter().filter(|f| f["exists"] == true).collect();
    let enabled = !existing.is_empty() && existing.iter().all(|f| f["packetSendRate"].as_f64().unwrap_or(0.0) > 0.0);
    json!({
        "installDir": install_dir,
        "valid": true,
        "enabled": enabled,
        "files": files,
        "installs": installs,
        "activePlatform": active_platform,
        "runningGame": running_info,
    })
}

pub fn enable_stats_api(install_dir: &str, documents_dir: Option<&Path>, packet_send_rate: u32, port: u16) -> io::Result<Vec<PathBuf>> {
    let mut written = vec![];
    let installs = detect_game_installations();
    let mut all_dirs = vec![install_dir.to_string()];
    if let Some(e) = installs.epic {
        all_dirs.push(e);
    }
    if let Some(s) = installs.steam {
        all_dirs.push(s);
    }
    if let Some(info) = get_running_rocket_league_info() {
        if let Some(dir) = info.install_dir {
            all_dirs.push(dir);
        }
    }

    let mut target_paths = vec![];
    for d in all_dirs {
        target_paths.extend(target_files(&d, documents_dir));
    }
    let mut seen = std::collections::HashSet::new();
    let unique_targets: Vec<PathBuf> = target_paths.into_iter().filter(|f| seen.insert(f.to_string_lossy().to_lowercase())).collect();

    for path in unique_targets {
        let backup = PathBuf::from(format!("{}.bak", path.display()));
        if path.exists() && !backup.exists() {
            fs::copy(&path, &backup)?;
        }
        let (text, encoding) = read_ini(&path);
        let updated = set_ini_values(&text, &[("PacketSendRate", packet_send_rate.to_string()), ("Port", port.to_string())]);
        write_ini(&path, &updated, encoding)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(windows)]
pub fn is_rocket_league_running() -> bool {
    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut std::ffi::c_void;
        fn Process32FirstW(h_snapshot: *mut std::ffi::c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: *mut std::ffi::c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn CloseHandle(h_object: *mut std::ffi::c_void) -> i32;
    }

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(0x00000002 /* TH32CS_SNAPPROCESS */, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return false;
        }

        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut found = false;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(entry.sz_exe_file.len());
                let exe = String::from_utf16_lossy(&entry.sz_exe_file[..len]).to_lowercase();
                if exe == "rocketleague.exe" {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}

/// Récupère le chemin d'exécution et la plateforme de Rocket League lorsqu'il tourne (comme BakkesMod).
#[cfg(windows)]
pub fn get_running_rocket_league_info() -> Option<RunningGameInfo> {
    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut std::ffi::c_void;
        fn Process32FirstW(h_snapshot: *mut std::ffi::c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: *mut std::ffi::c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(h_process: *mut std::ffi::c_void, dw_flags: u32, lp_exe_name: *mut u16, lpdw_size: *mut u32) -> i32;
        fn CloseHandle(h_object: *mut std::ffi::c_void) -> i32;
    }

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(0x00000002 /* TH32CS_SNAPPROCESS */, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return None;
        }

        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dw_size = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut found_pid: Option<u32> = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(entry.sz_exe_file.len());
                let exe = String::from_utf16_lossy(&entry.sz_exe_file[..len]).to_lowercase();
                if exe == "rocketleague.exe" {
                    found_pid = Some(entry.th32_process_id);
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        let pid = found_pid?;
        let handle = OpenProcess(0x1000 /* PROCESS_QUERY_LIMITED_INFORMATION */, 0, pid);
        if handle.is_null() {
            return None;
        }

        let mut path_buf = [0u16; 1024];
        let mut size = path_buf.len() as u32;
        let success = QueryFullProcessImageNameW(handle, 0, path_buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);

        if success != 0 && size > 0 {
            let full_exe_path = String::from_utf16_lossy(&path_buf[..size as usize]);
            let path_lower = full_exe_path.to_lowercase();
            let platform = if path_lower.contains("steam") {
                "steam".to_string()
            } else if path_lower.contains("epic") {
                "epic".to_string()
            } else {
                "unknown".to_string()
            };

            let exe_path_buf = PathBuf::from(&full_exe_path);
            let install_dir = normalize_install_dir(&exe_path_buf)
                .map(|p| p.to_string_lossy().into_owned());

            return Some(RunningGameInfo {
                pid,
                exe_path: full_exe_path,
                install_dir,
                platform,
            });
        }
        None
    }
}

#[cfg(not(windows))]
pub fn get_running_rocket_league_info() -> Option<RunningGameInfo> {
    None
}

/// Vérifie si la fenêtre active au premier plan appartient à Rocket League.
#[cfg(windows)]
pub fn is_rocket_league_focused() -> bool {
    extern "system" {
        fn GetForegroundWindow() -> *mut std::ffi::c_void;
        fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, lpdw_process_id: *mut u32) -> u32;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(h_process: *mut std::ffi::c_void, dw_flags: u32, lp_exe_name: *mut u16, lpdw_size: *mut u32) -> i32;
        fn GetClassNameW(hwnd: *mut std::ffi::c_void, lp_class_name: *mut u16, n_max_count: i32) -> i32;
        fn GetWindowTextW(hwnd: *mut std::ffi::c_void, lp_string: *mut u16, n_max_count: i32) -> i32;
        fn CloseHandle(h_object: *mut std::ffi::c_void) -> i32;
    }

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }

        // 1. Vérification rapide par classe de fenêtre Unreal Engine 3 de Rocket League
        let mut class_buf = [0u16; 128];
        let class_len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
        if class_len > 0 {
            let class_str = String::from_utf16_lossy(&class_buf[..class_len as usize]);
            if class_str == "LaunchUnrealUWindowsClient" {
                return true;
            }
        }

        // 2. Vérification par titre de la fenêtre
        let mut title_buf = [0u16; 128];
        let title_len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        if title_len > 0 {
            let title_str = String::from_utf16_lossy(&title_buf[..title_len as usize]).to_lowercase();
            if title_str.starts_with("rocket league") {
                return true;
            }
        }

        // 3. Vérification par nom de l'exécutable du processus
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != 0 {
            let handle = OpenProcess(0x1000 /* PROCESS_QUERY_LIMITED_INFORMATION */, 0, pid);
            if !handle.is_null() {
                let mut path_buf = [0u16; 512];
                let mut size = path_buf.len() as u32;
                let success = QueryFullProcessImageNameW(handle, 0, path_buf.as_mut_ptr(), &mut size);
                CloseHandle(handle);
                if success != 0 && size > 0 {
                    let path = String::from_utf16_lossy(&path_buf[..size as usize]).to_lowercase();
                    if path.ends_with("rocketleague.exe") {
                        return true;
                    }
                }
            }
        }

        false
    }
}

#[cfg(not(windows))]
pub fn is_rocket_league_running() -> bool {
    false
}

#[cfg(not(windows))]
pub fn is_rocket_league_focused() -> bool {
    true
}
