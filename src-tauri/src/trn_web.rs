//! Accès à l'API interne de Tracker Network depuis une fenêtre WebView2 masquée.
//!
//! Le site bloque les clients HTTP nus (Cloudflare) : on charge la page publique du profil dans
//! une vraie WebView, puis un script *de cette page* interroge l'API et dépose le JSON dans le
//! fragment d'URL, que l'on relit ici. Une seule implémentation sert la synchronisation du MMR de
//! la session et la récupération des profils du lobby (LobbyTracker).

use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

const OFFSCREEN: f64 = -32000.0;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_TICKS: u32 = 50;

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(0);

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

/// Script exécuté dans la page masquée : interroge `api_url` et écrit le résultat dans le
/// fragment de l'URL (`#TRN_SUCCESS=…` ou `#TRN_ERROR=…`).
fn init_script(api_url: &str) -> String {
    let api = serde_json::to_string(api_url).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"
(function() {{
  try {{
    const s = document.createElement('style');
    s.innerHTML = 'html, body {{ display: none !important; opacity: 0 !important; visibility: hidden !important; }}';
    (document.head || document.documentElement).appendChild(s);
  }} catch (e) {{}}

  const API = {api};
  let finished = false;
  function report(success, data) {{
    if (finished) return;
    finished = true;
    window.location.hash = (success ? 'TRN_SUCCESS=' : 'TRN_ERROR=') + encodeURIComponent(JSON.stringify(data));
  }}

  function tryFetch() {{
    try {{
      const sep = API.includes('?') ? '&' : '?';
      fetch(API + sep + '_nocache=' + Date.now(), {{
        cache: 'no-store',
        headers: {{
          'Accept': 'application/json',
          'Cache-Control': 'no-cache, no-store, must-revalidate',
          'Pragma': 'no-cache'
        }},
        credentials: 'include'
      }})
      .then(r => {{
        if (r.ok) return r.json();
        throw new Error('HTTP ' + r.status);
      }})
      .then(data => {{
        if (data && data.data) report(true, data);
      }})
      .catch(() => {{}});
    }} catch (e) {{}}
  }}

  function tryDom() {{
    try {{
      if (document.body && (document.body.innerText.includes('Player Not Found') || document.body.innerText.includes('We could not find'))) {{
        report(false, 'Profil introuvable sur Rocket League Tracker');
      }}
    }} catch (e) {{}}
  }}

  let attempts = 0;
  const timer = setInterval(() => {{
    attempts++;
    tryFetch();
    tryDom();
    if (attempts > 30 || finished) clearInterval(timer);
  }}, 800);
  setTimeout(tryFetch, 400);
}})();
"#
    )
}

/// Charge `page_url` dans une WebView masquée puis renvoie le JSON complet de `api_url`.
pub async fn fetch_json(app: &AppHandle, page_url: &str, api_url: &str) -> Result<Value, String> {
    let parsed = page_url.parse::<tauri::Url>().map_err(|e| format!("URL invalide : {e}"))?;
    let label = format!("tracker_sync_{}", WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed));
    let script = init_script(api_url);
    let app = app.clone();

    // Le sondage dort entre deux lectures : hors du thread de l'exécuteur asynchrone.
    tauri::async_runtime::spawn_blocking(move || {
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
            .initialization_script(&script)
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
    .map_err(|e| e.to_string())?
}
