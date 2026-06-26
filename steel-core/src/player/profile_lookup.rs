//! Online player profile lookup.

use reqwest::{StatusCode, Url};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::config::AuthServiceConfig;
use crate::player::known_players::KnownPlayer;

/// Error returned when resolving a player profile from the configured auth service.
#[derive(Debug, Error)]
pub enum ProfileLookupError {
    /// The profile service does not know this player.
    #[error("Unknown player {0}")]
    UnknownPlayer(String),
    /// The configured profile service host is invalid.
    #[error("Invalid profile service URL configured: {0}")]
    InvalidProfilesHost(String),
    /// The profile service request failed before a response was received.
    #[error("Profile lookup failed for {name}: {source}")]
    Request {
        /// Requested player name.
        name: String,
        /// Transport error.
        source: reqwest::Error,
    },
    /// The profile service returned a non-success status other than a not-found response.
    #[error("Profile lookup service returned status {status} for {name}")]
    ServiceResponse {
        /// Requested player name.
        name: String,
        /// HTTP status returned by the service.
        status: StatusCode,
    },
    /// The profile service returned a malformed response.
    #[error("Invalid profile lookup response for {name}: {reason}")]
    InvalidResponse {
        /// Requested player name.
        name: String,
        /// Response validation failure.
        reason: String,
    },
}

#[derive(Deserialize)]
struct ProfileLookupResponse {
    id: String,
    name: String,
}

/// Looks up a player profile by name through the configured profile service.
///
/// The caller is responsible for checking local caches and offline-mode behavior first.
pub async fn lookup_online_profile(
    client: &reqwest::Client,
    auth: &AuthServiceConfig,
    name: &str,
) -> Result<KnownPlayer, ProfileLookupError> {
    let lookup_name = name.to_ascii_lowercase();
    let url = profile_lookup_url(auth, &lookup_name)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|source| ProfileLookupError::Request {
            name: name.to_owned(),
            source,
        })?;

    match response.status() {
        StatusCode::OK => parse_profile_response(response, name).await,
        StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => {
            Err(ProfileLookupError::UnknownPlayer(name.to_owned()))
        }
        status => Err(ProfileLookupError::ServiceResponse {
            name: name.to_owned(),
            status,
        }),
    }
}

fn profile_lookup_url(
    auth: &AuthServiceConfig,
    normalized_name: &str,
) -> Result<Url, ProfileLookupError> {
    let endpoint = format!(
        "{}/minecraft/profile/lookup/name/{normalized_name}",
        auth.profiles_host().trim_end_matches('/')
    );
    Url::parse(&endpoint).map_err(|_| ProfileLookupError::InvalidProfilesHost(endpoint))
}

async fn parse_profile_response(
    response: reqwest::Response,
    requested_name: &str,
) -> Result<KnownPlayer, ProfileLookupError> {
    let profile = response
        .json::<ProfileLookupResponse>()
        .await
        .map_err(|source| ProfileLookupError::InvalidResponse {
            name: requested_name.to_owned(),
            reason: source.to_string(),
        })?;
    let uuid =
        Uuid::parse_str(&profile.id).map_err(|source| ProfileLookupError::InvalidResponse {
            name: requested_name.to_owned(),
            reason: source.to_string(),
        })?;

    Ok(KnownPlayer::new(uuid, profile.name))
}

#[cfg(test)]
mod tests {
    use super::profile_lookup_url;
    use crate::config::AuthServiceConfig;

    #[test]
    fn profile_lookup_url_uses_mojang_profiles_host_by_default() {
        let url = profile_lookup_url(&AuthServiceConfig::default(), "steve").expect("URL builds");

        assert_eq!(
            url.as_str(),
            "https://api.mojang.com/minecraft/profile/lookup/name/steve"
        );
    }

    #[test]
    fn profile_lookup_url_uses_configured_profiles_host() {
        let auth = AuthServiceConfig {
            profiles_host: "https://profiles.example.com".to_owned(),
            ..AuthServiceConfig::default()
        };
        let url = profile_lookup_url(&auth, "steve").expect("URL builds");

        assert_eq!(
            url.as_str(),
            "https://profiles.example.com/minecraft/profile/lookup/name/steve"
        );
    }
}
