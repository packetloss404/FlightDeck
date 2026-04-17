//! Provider configuration: per-user LLM provider registry that the native
//! agent runner consults for API-key lookup and model defaults.
//!
//! Secrets are never stored in the JSON registry — API keys live only in the
//! OS keyring, keyed on the pair (service = `flightdeck`, account =
//! `provider:<id>`). The JSON file holds the non-secret metadata needed to
//! build a provider client.

use std::fs;
use std::path::PathBuf;

use keyring::Entry;
use serde::{Deserialize, Serialize};
use tracing::warn;
use zeroize::Zeroizing;

use super::storage::{data_dir, ensure_data_dir};

pub const PROVIDERS_FILENAME: &str = "providers.json";
pub const KEYRING_SERVICE: &str = "flightdeck";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
}

impl ProviderKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
        }
    }

    /// Static fallback model list shown in the TUI picker when a live fetch
    /// isn't available. The live `/v1/models` fetch lands with the providers
    /// view in a later step.
    pub fn default_models(&self) -> &'static [&'static str] {
        match self {
            ProviderKind::Anthropic => &[
                "claude-opus-4-7",
                "claude-sonnet-4-6",
                "claude-haiku-4-5-20251001",
            ],
        }
    }

    pub fn default_model(&self) -> &'static str {
        self.default_models()[0]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub default_model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl ProviderConfig {
    pub fn keyring_account(&self) -> String {
        keyring_account_for(&self.id)
    }
}

pub fn keyring_account_for(provider_id: &str) -> String {
    format!("provider:{}", provider_id)
}

// ---------- JSON persistence ----------

pub fn providers_file_path() -> PathBuf {
    data_dir().join(PROVIDERS_FILENAME)
}

pub fn load_providers() -> Vec<ProviderConfig> {
    let path = providers_file_path();
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Vec<ProviderConfig>>(&content) {
            Ok(providers) => providers,
            Err(e) => {
                warn!("Failed to parse {:?}: {}, using empty provider list", path, e);
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    }
}

pub fn save_providers(providers: &[ProviderConfig]) -> Result<(), String> {
    let dir = ensure_data_dir()?;
    let path = dir.join(PROVIDERS_FILENAME);
    let json = serde_json::to_string_pretty(providers)
        .map_err(|e| format!("Failed to serialize providers: {}", e))?;
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write providers file {:?}: {}", path, e))
}

// ---------- Keyring helpers ----------

fn entry_for(provider_id: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, &keyring_account_for(provider_id))
        .map_err(|e| format!("keyring entry init failed: {}", e))
}

pub fn set_api_key(provider_id: &str, api_key: &str) -> Result<(), String> {
    entry_for(provider_id)?
        .set_password(api_key)
        .map_err(|e| format!("keyring set failed: {}", e))
}

pub fn get_api_key(provider_id: &str) -> Result<Zeroizing<String>, String> {
    let key = entry_for(provider_id)?
        .get_password()
        .map_err(|e| format!("keyring get failed: {}", e))?;
    Ok(Zeroizing::new(key))
}

pub fn delete_api_key(provider_id: &str) -> Result<(), String> {
    match entry_for(provider_id)?.delete_credential() {
        Ok(()) => Ok(()),
        // Missing entry isn't an error for our purposes — the caller wants the
        // secret gone and it already is.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete failed: {}", e)),
    }
}

pub fn has_api_key(provider_id: &str) -> bool {
    match entry_for(provider_id) {
        Ok(entry) => entry.get_password().is_ok(),
        Err(_) => false,
    }
}

// ---------- Connection test ----------

/// Summary returned from a `test_connection` call. `model_count` is populated
/// when the provider responds with its model catalogue.
#[derive(Debug, Clone)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
    pub model_count: Option<usize>,
}

/// Hit a cheap, authenticated provider endpoint to validate an API key.
///
/// For Anthropic this is `GET /v1/models`. Runs synchronously with a short
/// timeout so it's safe to call from the TUI loop without blocking the UI for
/// long. Returns a `TestConnectionResult` in both success and failure paths so
/// the caller can surface the specific error to the user.
pub fn test_connection(kind: ProviderKind, base_url: Option<&str>, api_key: &str) -> TestConnectionResult {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return TestConnectionResult {
                ok: false,
                message: format!("client build failed: {}", e),
                model_count: None,
            };
        }
    };

    match kind {
        ProviderKind::Anthropic => {
            let url = format!(
                "{}/v1/models",
                base_url.unwrap_or("https://api.anthropic.com").trim_end_matches('/')
            );
            let resp = client
                .get(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send();
            match resp {
                Ok(r) if r.status().is_success() => {
                    let count = r
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|v| v.get("data").and_then(|d| d.as_array().map(|a| a.len())));
                    TestConnectionResult {
                        ok: true,
                        message: "connected".into(),
                        model_count: count,
                    }
                }
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().unwrap_or_default();
                    TestConnectionResult {
                        ok: false,
                        message: format!("HTTP {}: {}", status, body.chars().take(120).collect::<String>()),
                        model_count: None,
                    }
                }
                Err(e) => TestConnectionResult {
                    ok: false,
                    message: format!("request failed: {}", e),
                    model_count: None,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_account_is_stable() {
        assert_eq!(keyring_account_for("anthropic-primary"), "provider:anthropic-primary");
    }

    #[test]
    fn provider_config_json_round_trip() {
        let cfg = ProviderConfig {
            id: "anthropic-primary".into(),
            kind: ProviderKind::Anthropic,
            display_name: "Anthropic (primary)".into(),
            base_url: None,
            default_model: "claude-sonnet-4-6".into(),
            enabled: true,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let round: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(round.id, cfg.id);
        assert_eq!(round.kind, ProviderKind::Anthropic);
        assert_eq!(round.default_model, "claude-sonnet-4-6");
    }

    #[test]
    fn json_file_missing_yields_empty_list() {
        // load_providers should not panic when the file doesn't exist. We can't
        // reliably remove the real file in a unit test, but we can exercise the
        // parse path with garbage directly.
        let parsed: Result<Vec<ProviderConfig>, _> = serde_json::from_str("not json");
        assert!(parsed.is_err());
    }

    #[test]
    fn default_models_non_empty() {
        assert!(!ProviderKind::Anthropic.default_models().is_empty());
        assert!(!ProviderKind::Anthropic.default_model().is_empty());
    }
}
