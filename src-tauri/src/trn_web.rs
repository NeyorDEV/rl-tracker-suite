//! Accès à l'API interne de Tracker Network depuis une fenêtre WebView2 masquée.
//!
//! Le site bloque les clients HTTP nus (Cloudflare) : on ouvre l'URL de l'API dans une vraie
//! WebView, qui passe le défi comme un navigateur. Un script de cette fenêtre lit le JSON affiché
//! et le dépose dans le fragment d'URL, que l'on relit ici. Une seule implémentation sert la
//! synchronisation du MMR de la session et les profils du lobby (LobbyTracker).
//!
//! Garde-fous pour ne jamais submerger Tracker Network (une seule requête par recherche, jamais
//! de boucle de relance) :
//! - les recherches passent l'une après l'autre, avec un délai minimum entre deux ;
//! - quand le site répond « trop de requêtes » (429), les recherches *automatiques* attendent un
//!   court moment, de plus en plus long si le blocage persiste (30 s, 1 min, 2 min… 5 min max) ;
//! - une recherche demandée par l'utilisateur (bouton) n'attend jamais : elle réessaie tout de suite.

use serde_json::Value;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use tauri::Manager;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

const OFFSCREEN: f64 = -32000.0;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_TICKS: u32 = 50;
/// Délai minimum entre la fin d'une recherche et le début de la suivante.
const MIN_GAP: Duration = Duration::from_millis(2000);
/// Pause des recherches automatiques après la première réponse 429 (doublée à chaque nouvelle).
const FIRST_PAUSE_SECS: i64 = 30;
const MAX_PAUSE_SECS: i64 = 300;

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(0);
/// Les recherches se font une par une, quel que soit l'appelant (session ou lobby).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LAST_END: Mutex<Option<Instant>> = Mutex::new(None);
/// Heure Unix jusqu'à laquelle les recherches automatiques attendent (0 = pas de pause).
static PAUSED_UNTIL: AtomicI64 = AtomicI64::new(0);
/// Nombre de réponses 429 d'affilée (remis à zéro par une réponse normale).
static STRIKES: AtomicU32 = AtomicU32::new(0);

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Secondes restantes de la pause, si elle est en cours.
fn pause_remaining(until: i64, now: i64) -> Option<i64> {
    (until > now).then(|| until - now)
}

/// Durée de la pause après `strikes` réponses 429 d'affilée : 30 s, 60 s, 120 s, 240 s, puis 300 s.
fn pause_secs(strikes: u32) -> i64 {
    (FIRST_PAUSE_SECS << strikes.saturating_sub(1).min(8)).min(MAX_PAUSE_SECS)
}

fn pause_message(remaining_secs: i64) -> String {
    format!(
        "Tracker Network limite les requêtes (erreur 429) : nouvel essai automatique dans {} s. Le bouton de synchronisation réessaie tout de suite.",
        remaining_secs.max(1)
    )
}

/// Décode un fragment d'URL (`%XX`, `+` = espace).
pub fn urlencoding_decode(input: &str) -> String {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Script exécuté dans la fenêtre masquée, qui affiche directement le JSON de l'API : il attend
/// que le défi Cloudflare soit passé, lit le texte de la page (aucune requête supplémentaire) et
/// écrit le résultat dans le fragment de l'URL (`#TRN_SUCCESS=…` ou `#TRN_ERROR=…`).
const INIT_SCRIPT: &str = r#"
(function() {
  if (window.top !== window || window.__rltsArmed) return;
  window.__rltsArmed = true;

  let finished = false;
  let timer = null;
  function report(success, data) {
    if (finished) return;
    finished = true;
    clearInterval(timer);
    window.location.hash = (success ? 'TRN_SUCCESS=' : 'TRN_ERROR=') + encodeURIComponent(JSON.stringify(data));
  }

  // Code HTTP réel de la page (0 si le navigateur ne le donne pas).
  function httpStatus() {
    try {
      const nav = performance.getEntriesByType('navigation')[0];
      return (nav && nav.responseStatus) || 0;
    } catch (e) { return 0; }
  }

  function check() {
    try {
      const status = httpStatus();
      if (status === 429) return report(false, 'HTTP 429');
      const txt = ((document.body && document.body.innerText) || '').trim();
      if (txt.charAt(0) === '{') {
        let json = null;
        try { json = JSON.parse(txt); } catch (e) { return; }
        if (json.data) return report(true, json);
        const errors = JSON.stringify(json.errors || json).toLowerCase();
        if (status === 404 || errors.includes('notfound') || errors.includes('not found')) return report(false, 'Profil introuvable sur Rocket League Tracker');
        if (status >= 400 && status !== 403) return report(false, 'HTTP ' + status);
        return report(false, 'Réponse inattendue : ' + txt.slice(0, 160));
      }
      // Navigateur sans code de réponse : repli sur le texte de la page d'erreur Cloudflare.
      if (!status && /too many requests|error 1015/i.test(txt)) return report(false, 'HTTP 429');
    } catch (e) {}
  }

  timer = setInterval(check, 600);
})();
"#;

/// Recherche automatique (lobby, synchronisation en arrière-plan) : respecte la pause après un 429.
pub async fn fetch_json(app: &AppHandle, api_url: &str) -> Result<Value, String> {
    fetch(app, api_url, true).await
}

/// Recherche demandée par l'utilisateur (bouton) : ne reste jamais bloquée par la pause.
pub async fn fetch_json_manual(app: &AppHandle, api_url: &str) -> Result<Value, String> {
    fetch(app, api_url, false).await
}

/// Ligne du journal : la raison exacte, sans l'adresse complète.
fn log_failure(app: &AppHandle, api_url: &str, reason: &str) {
    let what = api_url.split_once("/profile/").map_or(api_url, |(_, t)| t);
    if let Ok(core) = app.state::<crate::Ctx>().core.lock() {
        core.log(&format!("Tracker Network ({what}) : {reason}"));
    }
}

async fn fetch(app: &AppHandle, api_url: &str, honor_pause: bool) -> Result<Value, String> {
    let parsed = api_url.parse::<tauri::Url>().map_err(|e| format!("URL invalide : {e}"))?;

    let _one_at_a_time = SERIAL.lock().await;
    if honor_pause {
        if let Some(left) = pause_remaining(PAUSED_UNTIL.load(Ordering::SeqCst), unix_now()) {
            return Err(pause_message(left));
        }
    }
    let wait = LAST_END
        .lock()
        .ok()
        .and_then(|t| t.map(|end| MIN_GAP.saturating_sub(end.elapsed())))
        .unwrap_or_default();
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }

    let label = format!("tracker_sync_{}", WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed));
    let task_app = app.clone();
    // Le sondage dort entre deux lectures : hors du thread de l'exécuteur asynchrone.
    let result = tauri::async_runtime::spawn_blocking(move || {
        let win = WebviewWindowBuilder::new(&task_app, &label, WebviewUrl::External(parsed))
            .title("RL Tracker Sync")
            .inner_size(1.0, 1.0)
            .position(OFFSCREEN, OFFSCREEN)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .skip_taskbar(true)
            .resizable(true)
            .focused(false)
            .visible(false)
            .initialization_script(INIT_SCRIPT)
            .build()
            .map_err(|e| format!("Impossible d'ouvrir le navigateur en arrière-plan : {e}"))?;
        let _ = win.hide();

        let mut outcome: Option<Result<Value, String>> = None;
        for _ in 0..MAX_TICKS {
            std::thread::sleep(POLL_INTERVAL);
            let Ok(url) = win.url() else { continue };
            let s = url.as_str();
            if let Some(pos) = s.find("#TRN_SUCCESS=") {
                let decoded = urlencoding_decode(&s[pos + "#TRN_SUCCESS=".len()..]);
                if let Ok(val) = serde_json::from_str::<Value>(&decoded) {
                    outcome = Some(Ok(val));
                    break;
                }
            } else if let Some(pos) = s.find("#TRN_ERROR=") {
                let msg = urlencoding_decode(&s[pos + "#TRN_ERROR=".len()..]);
                outcome = Some(Err(serde_json::from_str::<String>(&msg).unwrap_or(msg)));
                break;
            }
        }
        let _ = win.destroy();
        outcome.unwrap_or_else(|| Err("Délai de synchronisation dépassé ou profil inaccessible.".into()))
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Ok(mut last) = LAST_END.lock() {
        *last = Some(Instant::now());
    }
    match result {
        Ok(v) => {
            STRIKES.store(0, Ordering::SeqCst);
            PAUSED_UNTIL.store(0, Ordering::SeqCst);
            Ok(v)
        }
        Err(e) if e.contains("HTTP 429") => {
            let strikes = STRIKES.fetch_add(1, Ordering::SeqCst) + 1;
            let secs = pause_secs(strikes);
            PAUSED_UNTIL.store(unix_now() + secs, Ordering::SeqCst);
            log_failure(app, api_url, &format!("429 (trop de requêtes), pause automatique {secs} s"));
            Err(pause_message(secs))
        }
        Err(e) => {
            log_failure(app, api_url, &e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_counts_down_then_ends() {
        assert_eq!(pause_remaining(1_000, 400), Some(600));
        assert_eq!(pause_remaining(1_000, 1_000), None);
        assert_eq!(pause_remaining(0, 400), None);
        assert!(pause_message(45).contains("45 s"));
    }

    #[test]
    fn pause_grows_with_repeated_blocks_but_stays_short() {
        let secs: Vec<i64> = (1..=7).map(pause_secs).collect();
        assert_eq!(secs, vec![30, 60, 120, 240, 300, 300, 300]);
        assert_eq!(pause_secs(0), 30);
        assert_eq!(pause_secs(u32::MAX), 300);
    }

    #[test]
    fn script_uses_the_real_http_status() {
        assert!(INIT_SCRIPT.contains("responseStatus"));
        assert!(!INIT_SCRIPT.contains("errors.includes('429')"), "plus de détection par mot dans le JSON");
    }

    #[test]
    fn decodes_fragments() {
        assert_eq!(urlencoding_decode("a%20b+c%C3%A9"), "a b cé");
    }

    #[test]
    fn script_never_loops_on_the_api() {
        // Aucune requête réseau dans le script : il lit uniquement la page déjà affichée.
        assert!(!INIT_SCRIPT.contains("fetch("));
        assert!(INIT_SCRIPT.contains("window.top !== window"));
    }
}
