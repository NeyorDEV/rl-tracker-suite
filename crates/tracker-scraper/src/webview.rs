//! Source « WebView2 invisible » : une vraie fenêtre de navigateur résout le défi Cloudflare,
//! puis lit le JSON de l'API interne de Tracker Network depuis *sa propre* page web.
//! Rien n'est injecté dans le processus du jeu.
//!
//! La fenêtre masquée est ouverte par l'application (`WebFetch`), la même que celle qui
//! synchronise le MMR de la session : ce crate n'en contient pas de seconde copie.

use crate::fetcher::JsonSource;
use crate::ScrapeError;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracker_core::PlayerId;

const API_BASE: &str = "https://api.tracker.gg/api/v2/rocket-league/standard/profile";
const PAGE_BASE: &str = "https://rocketleague.tracker.network/rocket-league/profile";

/// Implémenté par l'application : charge `page_url` dans une WebView masquée (ce qui passe
/// Cloudflare) puis renvoie la réponse JSON de `api_url`.
#[async_trait]
pub trait WebFetch: Send + Sync {
    async fn get_json(&self, page_url: &str, api_url: &str) -> Result<Value, ScrapeError>;
}

pub struct WebSource {
    fetch: Arc<dyn WebFetch>,
}

impl WebSource {
    pub fn new(fetch: Arc<dyn WebFetch>) -> Self {
        Self { fetch }
    }
}

/// Page publique correspondant à une URL d'API (`…/profile/epic/pseudo[/sessions]`).
fn page_url_for(api_url: &str) -> String {
    let tail = api_url.split_once("/profile/").map_or("", |(_, t)| t);
    let mut segments = tail.split('/');
    let (platform, handle) = (segments.next().unwrap_or(""), segments.next().unwrap_or(""));
    format!("{PAGE_BASE}/{platform}/{handle}")
}

#[async_trait]
impl JsonSource for WebSource {
    fn name(&self) -> &'static str {
        "webview"
    }

    fn profile_url(&self, id: &PlayerId, handle: &str) -> String {
        format!("{API_BASE}/{}/{handle}", id.platform.tracker_slug())
    }

    fn sessions_url(&self, id: &PlayerId, handle: &str) -> String {
        format!("{}/sessions", self.profile_url(id, handle))
    }

    async fn get_json(&self, url: &str) -> Result<Value, ScrapeError> {
        self.fetch.get_json(&page_url_for(url), url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use tracker_core::Platform;

    struct Fake(Mutex<Vec<(String, String)>>);

    #[async_trait]
    impl WebFetch for Fake {
        async fn get_json(&self, page_url: &str, api_url: &str) -> Result<Value, ScrapeError> {
            self.0.lock().unwrap().push((page_url.into(), api_url.into()));
            Ok(json!({"data": {"segments": []}}))
        }
    }

    #[tokio::test]
    async fn profile_and_sessions_load_the_public_page_first() {
        let fake = Arc::new(Fake(Mutex::default()));
        let src = WebSource::new(fake.clone());
        let id = PlayerId::new(Platform::Epic, "6897812333f244858dddff1fbb06554f");
        let profile = src.profile_url(&id, "neyor.");
        assert!(src.get_json(&profile).await.unwrap().get("data").is_some());
        src.get_json(&src.sessions_url(&id, "neyor.")).await.unwrap();
        let calls = fake.0.lock().unwrap();
        assert_eq!(calls[0].0, "https://rocketleague.tracker.network/rocket-league/profile/epic/neyor.");
        assert_eq!(calls[0].1, "https://api.tracker.gg/api/v2/rocket-league/standard/profile/epic/neyor.");
        assert_eq!(calls[1].0, calls[0].0, "même page pour les sessions");
        assert!(calls[1].1.ends_with("/neyor./sessions"));
    }
}
