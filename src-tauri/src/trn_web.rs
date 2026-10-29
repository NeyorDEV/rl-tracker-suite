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
//! - dès que le site répond « trop de requêtes » (429), toutes les recherches s'arrêtent 10 minutes.

use serde_json::Value;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

const OFFSCREEN: f64 = -32000.0;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_TICKS: u32 = 50;
/// Délai minimum entre la fin d'une recherche et le début de la suivante.
const MIN_GAP: Duration = Duration::from_millis(2500);
/// Pause de toutes les recherches après une réponse 429.
const COOLDOWN_SECS: i64 = 600;

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(0);
/// Les recherches se font une par une, quel que soit l'appelant (session ou lobby).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LAST_END: Mutex<Option<Instant>> = Mutex::new(None);
/// Heure Unix jusqu'à laquelle plus aucune requête n'est envoyée (0 = pas de pause).
static PAUSED_UNTIL: AtomicI64 = AtomicI64::new(0);

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Secondes restantes de la pause, si elle est en cours.
fn pause_remaining(until: i64, now: i64) -> Option<i64> {
    (until > now).then(|| until - now)
}

fn pause_message(remaining_secs: i64) -> String {
    format!(
        "Tracker Network limite les requêtes (erreur 429) : recherches en pause encore {} min pour ne pas aggraver le blocage.",
        (remaining_secs + 59) / 60
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

  function check() {
    try {
      const txt = ((document.body && document.body.innerText) || '').trim();
      if (txt.charAt(0) === '{') {
        let json = null;
        try { json = JSON.parse(txt); } catch (e) { return; }
        if (json.data) return report(true, json);
        const errors = JSON.stringify(json.errors || json).toLowerCase();
        if (errors.includes('notfound') || errors.includes('not found')) return report(false, 'Profil introuvable sur Rocket League Tracker');
        if (errors.includes('toomany') || errors.includes('too many') || errors.includes('429')) return report(false, 'HTTP 429');
        return report(false, 'Réponse inattendue : ' + txt.slice(0, 160));
      }
      if (/too many requests|error 1015|rate limit/i.test(txt)) return report(false, 'HTTP 429');
    } catch (e) {}
  }

  timer = setInterval(check, 600);
})();
"#;

/// Ouvre `api_url` dans une WebView masquée et renvoie le JSON affiché.
pub async fn fetch_json(app: &AppHandle, api_url: &str) -> Result<Value, String> {
    let parsed = api_url.parse::<tauri::Url>().map_err(|e| format!("URL invalide : {e}"))?;

    let _one_at_a_time = SERIAL.lock().await;
    if let Some(left) = pause_remaining(PAUSED_UNTIL.load(Ordering::SeqCst), unix_now()) {
        return Err(pause_message(left));
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
    let app = app.clone();
    // Le sondage dort entre deux lectures : hors du thread de l'exécuteur asynchrone.
    let result = tauri::async_runtime::spawn_blocking(move || {
        let win = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed))
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
        Err(e) if e.contains("429") => {
            PAUSED_UNTIL.store(unix_now() + COOLDOWN_SECS, Ordering::SeqCst);
            Err(pause_message(COOLDOWN_SECS))
        }
        other => other,
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
        assert!(pause_message(600).contains("10 min"));
        assert!(pause_message(61).contains("2 min"));
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
