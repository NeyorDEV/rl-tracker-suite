use crate::{mapping, ScrapeError};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracker_core::{PlayerId, PlayerProfile};

#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// Récupère aussi l'historique de session : série de défaites et forme récente (requête en plus).
    pub with_sessions: bool,
    /// Pseudo du joueur : Tracker Network identifie les comptes Epic, Xbox et PSN par pseudo
    /// (seul Steam accepte l'identifiant numérique).
    pub lookup: Option<String>,
}

/// Segment d'URL identifiant le joueur chez Tracker Network, encodé pour une URL.
pub fn profile_handle(id: &PlayerId, lookup: Option<&str>) -> Result<String, ScrapeError> {
    use tracker_core::Platform;
    let raw = match id.platform {
        Platform::Steam => id.id.as_str(),
        _ => lookup.map(str::trim).filter(|n| !n.is_empty()).ok_or(ScrapeError::NameRequired)?,
    };
    Ok(percent_encode(raw))
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, id: &PlayerId, opts: FetchOptions) -> Result<PlayerProfile, ScrapeError>;
}

/// Source de JSON brut : WebView2 invisible, ou HTTP avec jeton TRN.
#[async_trait]
pub trait JsonSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn profile_url(&self, id: &PlayerId, handle: &str) -> String;
    fn sessions_url(&self, id: &PlayerId, handle: &str) -> String;
    async fn get_json(&self, url: &str) -> Result<Value, ScrapeError>;

    /// Deux ressources d'un coup : la seconde est un meilleur effort (`None` si elle échoue).
    /// Par défaut, deux appels ; la source WebView les lit dans une seule fenêtre.
    async fn get_json_pair(&self, url: &str, extra_url: &str) -> Result<(Value, Option<Value>), ScrapeError> {
        let main = self.get_json(url).await?;
        let extra = match self.get_json(extra_url).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::debug!("{} : ressource supplémentaire indisponible ({e})", self.name());
                None
            }
        };
        Ok((main, extra))
    }
}

/// Transforme une `JsonSource` en `Fetcher` (profil + sessions optionnelles).
pub struct SourceFetcher<S: JsonSource> {
    source: S,
}

impl<S: JsonSource> SourceFetcher<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

#[async_trait]
impl<S: JsonSource> Fetcher for SourceFetcher<S> {
    async fn fetch(&self, id: &PlayerId, opts: FetchOptions) -> Result<PlayerProfile, ScrapeError> {
        let handle = profile_handle(id, opts.lookup.as_deref())?;
        let profile_url = self.source.profile_url(id, &handle);
        // Profil et historique des sessions en un seul passage : une seule page à charger.
        // L'absence d'historique ne doit pas faire échouer le profil.
        let (json, sessions) = if opts.with_sessions {
            self.source.get_json_pair(&profile_url, &self.source.sessions_url(id, &handle)).await?
        } else {
            (self.source.get_json(&profile_url).await?, None)
        };
        let mut profile = mapping::parse_profile(&json, id)?;
        if let Some(s) = sessions {
            profile.loss_streak = mapping::parse_loss_streak(&s);
            profile.recent = mapping::parse_recent_form(&s);
        }
        Ok(profile)
    }
}

/// Essaie chaque source dans l'ordre et renvoie la première réussite.
pub struct HybridFetcher {
    fetchers: Vec<Arc<dyn Fetcher>>,
}

impl HybridFetcher {
    pub fn new(fetchers: Vec<Arc<dyn Fetcher>>) -> Self {
        Self { fetchers }
    }
}

#[async_trait]
impl Fetcher for HybridFetcher {
    async fn fetch(&self, id: &PlayerId, opts: FetchOptions) -> Result<PlayerProfile, ScrapeError> {
        // On garde l'erreur la plus parlante : la vraie raison de la première source (429, profil
        // introuvable, délai dépassé…), pas « aucune source » de la source de repli sans jeton.
        let mut reason = ScrapeError::NoSource;
        for f in &self.fetchers {
            match f.fetch(id, opts.clone()).await {
                Ok(p) => return Ok(p),
                Err(e) => {
                    tracing::debug!("source en échec pour {id} : {e}");
                    if matches!(reason, ScrapeError::NoSource) {
                        reason = e;
                    }
                }
            }
        }
        Err(reason)
    }
}

/// API publique officielle TRN, avec le jeton configuré par l'utilisateur.
pub struct TrnApiSource {
    client: reqwest::Client,
    key: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl TrnApiSource {
    /// `key` est relu à chaque requête pour suivre les changements de configuration.
    pub fn new(key: Arc<dyn Fn() -> Option<String> + Send + Sync>) -> Result<Self, ScrapeError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("LobbyTracker/0.1")
            .build()
            .map_err(|e| ScrapeError::Network(e.to_string()))?;
        Ok(Self { client, key })
    }
}

#[async_trait]
impl JsonSource for TrnApiSource {
    fn name(&self) -> &'static str {
        "trn-api"
    }

    fn profile_url(&self, id: &PlayerId, handle: &str) -> String {
        format!(
            "https://public-api.tracker.gg/v2/rocket-league/standard/profile/{}/{handle}",
            id.platform.tracker_slug()
        )
    }

    fn sessions_url(&self, id: &PlayerId, handle: &str) -> String {
        format!("{}/sessions", self.profile_url(id, handle))
    }

    async fn get_json(&self, url: &str) -> Result<Value, ScrapeError> {
        let key = (self.key)().filter(|k| !k.trim().is_empty()).ok_or(ScrapeError::NoSource)?;
        let resp = self
            .client
            .get(url)
            .header("TRN-Api-Key", key.trim())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| if e.is_timeout() { ScrapeError::Timeout } else { ScrapeError::Network(e.to_string()) })?;
        match resp.status().as_u16() {
            200..=299 => resp.json().await.map_err(|e| ScrapeError::BadPayload(e.to_string())),
            401 | 403 => Err(ScrapeError::Blocked),
            404 => Err(ScrapeError::NotFound),
            429 => Err(ScrapeError::RateLimited),
            c => Err(ScrapeError::Network(format!("HTTP {c}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracker_core::Platform;

    struct Stub(Result<(), ScrapeError>);

    #[async_trait]
    impl Fetcher for Stub {
        async fn fetch(&self, id: &PlayerId, _: FetchOptions) -> Result<PlayerProfile, ScrapeError> {
            match &self.0 {
                Ok(()) => Ok(PlayerProfile {
                    id: id.clone(),
                    display_name: "ok".into(),
                    total_wins: 0,
                    peak_tier: None,
                    playlists: vec![],
                    saves_per_match: None,
                    goals_per_match: None,
                    shots_per_match: None,
                    loss_streak: 0,
                    recent: None,
                    fetched_at: 0,
                }),
                Err(ScrapeError::Blocked) => Err(ScrapeError::Blocked),
                Err(ScrapeError::RateLimited) => Err(ScrapeError::RateLimited),
                Err(ScrapeError::NoSource) => Err(ScrapeError::NoSource),
                Err(_) => Err(ScrapeError::NotFound),
            }
        }
    }

    /// Source qui note chaque appel : un profil avec historique ne doit coûter qu'un seul passage.
    struct Counting(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl JsonSource for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn profile_url(&self, _: &PlayerId, h: &str) -> String {
            format!("profile/{h}")
        }
        fn sessions_url(&self, _: &PlayerId, h: &str) -> String {
            format!("profile/{h}/sessions")
        }
        async fn get_json(&self, url: &str) -> Result<Value, ScrapeError> {
            self.0.lock().unwrap().push(format!("single {url}"));
            Ok(serde_json::json!({"data": {"segments": []}}))
        }
        async fn get_json_pair(&self, url: &str, extra: &str) -> Result<(Value, Option<Value>), ScrapeError> {
            self.0.lock().unwrap().push(format!("pair {url} + {extra}"));
            Ok((
                serde_json::json!({"data": {"segments": []}}),
                Some(serde_json::json!({"data": {"items": [{"matches": [
                    {"metadata": {"result": "defeat", "playlist": "Ranked Doubles 2v2"}},
                    {"metadata": {"result": "defeat", "playlist": "Ranked Doubles 2v2"}},
                    {"metadata": {"result": "defeat", "playlist": "Ranked Doubles 2v2"}}]}]}})),
            ))
        }
    }

    #[tokio::test]
    async fn profile_and_sessions_use_a_single_pass() {
        let src = Arc::new(Counting(Default::default()));
        struct Shared(Arc<Counting>);
        #[async_trait]
        impl JsonSource for Shared {
            fn name(&self) -> &'static str { self.0.name() }
            fn profile_url(&self, i: &PlayerId, h: &str) -> String { self.0.profile_url(i, h) }
            fn sessions_url(&self, i: &PlayerId, h: &str) -> String { self.0.sessions_url(i, h) }
            async fn get_json(&self, u: &str) -> Result<Value, ScrapeError> { self.0.get_json(u).await }
            async fn get_json_pair(&self, u: &str, e: &str) -> Result<(Value, Option<Value>), ScrapeError> { self.0.get_json_pair(u, e).await }
        }
        let f = SourceFetcher::new(Shared(src.clone()));
        let id = PlayerId::new(Platform::Steam, "76561198012345678");
        let with = f.fetch(&id, FetchOptions { with_sessions: true, lookup: None }).await.unwrap();
        assert_eq!(with.loss_streak, 3, "l'historique est lu");
        f.fetch(&id, FetchOptions { with_sessions: false, lookup: None }).await.unwrap();
        let calls = src.0.lock().unwrap();
        assert_eq!(calls.len(), 2, "un passage par joueur, avec ou sans historique");
        assert!(calls[0].starts_with("pair ") && calls[1].starts_with("single "));
    }

    #[test]
    fn handle_uses_name_except_on_steam() {
        let epic = PlayerId::new(Platform::Epic, "6897812333f244858dddff1fbb06554f");
        assert!(matches!(profile_handle(&epic, None), Err(ScrapeError::NameRequired)));
        assert_eq!(profile_handle(&epic, Some("Neyor. Dev")).unwrap(), "Neyor.%20Dev");
        let steam = PlayerId::new(Platform::Steam, "76561198012345678");
        assert_eq!(profile_handle(&steam, Some("whatever")).unwrap(), "76561198012345678");
    }

    #[tokio::test]
    async fn hybrid_falls_back_to_next_source() {
        let id = PlayerId::new(Platform::Steam, "76561198012345678");
        let h = HybridFetcher::new(vec![Arc::new(Stub(Err(ScrapeError::Blocked))), Arc::new(Stub(Ok(())))]);
        assert_eq!(h.fetch(&id, FetchOptions::default()).await.unwrap().display_name, "ok");
    }

    #[tokio::test]
    async fn hybrid_reports_the_real_reason_and_empty_means_no_source() {
        let id = PlayerId::new(Platform::Steam, "76561198012345678");
        let h = HybridFetcher::new(vec![Arc::new(Stub(Err(ScrapeError::Blocked)))]);
        assert!(matches!(h.fetch(&id, FetchOptions::default()).await, Err(ScrapeError::Blocked)));
        // La source de repli sans jeton ne doit pas masquer la vraie raison de la première.
        let h = HybridFetcher::new(vec![
            Arc::new(Stub(Err(ScrapeError::RateLimited))),
            Arc::new(Stub(Err(ScrapeError::NoSource))),
        ]);
        assert!(matches!(h.fetch(&id, FetchOptions::default()).await, Err(ScrapeError::RateLimited)));
        let empty = HybridFetcher::new(vec![]);
        assert!(matches!(empty.fetch(&id, FetchOptions::default()).await, Err(ScrapeError::NoSource)));
    }
}
