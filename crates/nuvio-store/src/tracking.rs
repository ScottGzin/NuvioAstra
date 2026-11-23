//! Per-profile OAuth state for Trakt/Simkl. Tokens and pending PKCE/device material are kept in
//! Rust-owned documents, never returned to the UI.

use crate::nextgen::AppStore;
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackingToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackingPending {
    /// Trakt device code, or the Simkl PKCE verifier.
    pub secret: String,
    pub user_code: Option<String>,
    pub verification_url: Option<String>,
    /// Simkl `state` parameter, checked on the callback.
    pub state: Option<String>,
    pub interval_secs: i64,
    pub expires_at_epoch_ms: i64,
}

type ByProfile<T> = BTreeMap<i64, BTreeMap<String, T>>;

impl AppStore {
    pub fn tracking_token(&self, profile: i64, provider: &str) -> Result<Option<TrackingToken>, StoreError> {
        Ok(self.read::<ByProfile<TrackingToken>>("tracking-tokens")?.unwrap_or_default()
            .remove(&profile).and_then(|map| map.get(provider).cloned()))
    }

    pub fn set_tracking_token(&self, profile: i64, provider: &str, token: Option<TrackingToken>) -> Result<(), StoreError> {
        let mut all = self.read::<ByProfile<TrackingToken>>("tracking-tokens")?.unwrap_or_default();
        let entry = all.entry(profile).or_default();
        match token {
            Some(token) => { entry.insert(provider.to_string(), token); }
            None => { entry.remove(provider); }
        }
        self.write("tracking-tokens", &all)
    }

    pub fn tracking_pending(&self, profile: i64, provider: &str) -> Result<Option<TrackingPending>, StoreError> {
        Ok(self.read::<ByProfile<TrackingPending>>("tracking-pending")?.unwrap_or_default()
            .remove(&profile).and_then(|map| map.get(provider).cloned()))
    }

    pub fn set_tracking_pending(&self, profile: i64, provider: &str, pending: Option<TrackingPending>) -> Result<(), StoreError> {
        let mut all = self.read::<ByProfile<TrackingPending>>("tracking-pending")?.unwrap_or_default();
        let entry = all.entry(profile).or_default();
        match pending {
            Some(pending) => { entry.insert(provider.to_string(), pending); }
            None => { entry.remove(provider); }
        }
        self.write("tracking-pending", &all)
    }
}
