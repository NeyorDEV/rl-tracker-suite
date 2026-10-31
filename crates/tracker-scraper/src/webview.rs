//! Source « WebView2 invisible » : une vraie fenêtre de navigateur ouvre l'URL de l'API interne
//! de Tracker Network (ce qui résout le défi Cloudflare) et l'application lit le JSON affiché.
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

/// Implémenté par l'application : ouvre `api_url` dans une WebView masquée (une seule requête,
/// sans relance) et renvoie le JSON affiché.
#[async_trait]
pub trait WebFetch: Send + Sync {
    async fn get_json(&self, api_url: &str) -> Result<Value, ScrapeError>;

    /// Deux ressources de l'API dans une seule fenêtre (profil + historique des sessions) : la
    /// seconde est un meilleur effort.
    async fn get_json_pair(&self, api_url: &str, extra_url: &str) -> Result<(Value, Option<Value>), ScrapeError>;
}

pub struct WebSource {
    fetch: Arc<dyn WebFetch>,
}

impl WebSource {
    pub fn new(fetch: Arc<dyn WebFetch>) -> Self {
        Self { fetch }
    }
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
        self.fetch.get_json(url).await
    }

    async fn get_json_pair(&self, url: &str, extra_url: &str) -> Result<(Value, Option<Value>), ScrapeError> {
        self.fetch.get_json_pair(url, extra_url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use tracker_core::Platform;

    struct Fake(Mutex<Vec<String>>);

    #[async_trait]
    impl WebFetch for Fake {
        async fn get_json(&self, api_url: &str) -> Result<Value, ScrapeError> {
            self.0.lock().unwrap().push(api_url.into());
            Ok(json!({"data": {"segments": []}}))
        }

        async fn get_json_pair(&self, api_url: &str, extra_url: &str) -> Result<(Value, Option<Value>), ScrapeError> {
            self.0.lock().unwrap().push(format!("{api_url} + {extra_url}"));
            Ok((json!({"data": {"segments": []}}), Some(json!({"data": {"items": []}}))))
        }
    }

    #[tokio::test]
    async fn profile_and_sessions_are_one_request_each() {
        let fake = Arc::new(Fake(Mutex::default()));
        let src = WebSource::new(fake.clone());
        let id = PlayerId::new(Platform::Epic, "6897812333f244858dddff1fbb06554f");
        let profile = src.profile_url(&id, "neyor.");
        let sessions = src.sessions_url(&id, "neyor.");
        assert!(src.get_json(&profile).await.unwrap().get("data").is_some());
        src.get_json(&sessions).await.unwrap();
        {
            let calls = fake.0.lock().unwrap();
            assert_eq!(calls.len(), 2, "une requête par appel, aucune relance");
            assert_eq!(calls[0], "https://api.tracker.gg/api/v2/rocket-league/standard/profile/epic/neyor.");
            assert!(calls[1].ends_with("/neyor./sessions"));
        }
        // Profil + historique : un seul appel à l'application, donc une seule fenêtre.
        let (main, extra) = src.get_json_pair(&profile, &sessions).await.unwrap();
        assert!(main.get("data").is_some() && extra.is_some());
        assert_eq!(fake.0.lock().unwrap().len(), 3);
    }
}
