//! The Home hero's catalogs follow Nuvio Desktop on the same computer.
//!
//! Nuvio keeps that choice out of the account (`setHeroSourceEnabled(pushRemote = false)`; the home
//! payload has no field for it), so the only place it exists is its own settings file. It is read,
//! never written: a choice made in Nuvio is adopted here, one made here stays until Nuvio's changes.

use crate::nextgen::AppStore;
use crate::{LegacyStore, StoreError};
use serde::{Deserialize, Serialize};

/// What Nuvio had the last time it was read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Seen {
    hero_enabled: bool,
    /// Sorted: moving rows in Nuvio is not another choice.
    chosen: Vec<String>,
}

fn seen_doc(profile: i64) -> String {
    format!("nuvio-hero-seen-{profile}")
}

impl AppStore {
    /// Adopts what changed in Nuvio's hero settings since the last look; returns the profiles that
    /// changed here. Data of another account is left alone.
    pub fn follow_nuvio_hero(&self, nuvio: &LegacyStore) -> Result<Vec<i64>, StoreError> {
        if let Some(linked) = self.sync_state()?.linked_user_id {
            if nuvio.profiles()?.is_none_or(|p| p.user_id != linked) {
                return Ok(Vec::new());
            }
        }
        let mut changed = Vec::new();
        for index in self.profiles()?.profiles.iter().map(|p| p.index) {
            let Some(theirs) = nuvio.home_catalog_settings(index)? else { continue };
            if theirs.items.is_empty() {
                continue;
            }
            let mut rows: Vec<_> = theirs.items.iter().filter(|p| p.hero_source_enabled && !p.key.starts_with("collection_")).collect();
            rows.sort_by_key(|p| p.order);
            let in_order: Vec<String> = rows.into_iter().map(|p| p.key.clone()).collect();
            let mut chosen = in_order.clone();
            chosen.sort();
            let now = Seen { hero_enabled: theirs.hero_enabled, chosen };
            let before: Option<Seen> = self.read(&seen_doc(index))?;
            if before.as_ref() == Some(&now) {
                continue;
            }
            // The switch already existed here: the first look does not override it.
            let switched = before.as_ref().is_some_and(|b| b.hero_enabled != now.hero_enabled);
            let rechosen = before.as_ref().is_none_or(|b| b.chosen != now.chosen);
            let mut edited = false;
            self.update_profile_settings(index, |settings| {
                if switched && settings.home.hero_enabled != now.hero_enabled {
                    settings.home.hero_enabled = now.hero_enabled;
                    edited = true;
                }
                if rechosen && settings.home.hero_catalogs.as_ref() != Some(&in_order) {
                    settings.home.hero_catalogs = Some(in_order.clone());
                    edited = true;
                }
            })?;
            self.write(&seen_doc(index), &now)?;
            if edited {
                changed.push(index);
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_in_nuvio_is_adopted_and_one_here_stays_until_nuvio_changes() {
        let root = std::env::temp_dir().join(format!("nuvio-hero-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let nuvio_dir = root.join("Nuvio");
        std::fs::create_dir_all(&nuvio_dir).unwrap();
        let save = |enabled: bool, rows: &[(&str, bool, i64)]| {
            let items: Vec<_> = rows
                .iter()
                .map(|(key, hero, order)| serde_json::json!({ "key": key, "heroSourceEnabled": hero, "order": order }))
                .collect();
            let json = serde_json::json!({ "heroEnabled": enabled, "items": items }).to_string().replace(':', "\\:");
            std::fs::write(nuvio_dir.join("nuvio_home_catalog_settings.properties"), format!("home_catalog_settings_1={json}\n")).unwrap();
        };
        let store = AppStore::open(root.join("Astra")).unwrap();
        let nuvio = LegacyStore::open(&nuvio_dir).unwrap();
        let chosen = || store.profile_settings(1).unwrap().home.hero_catalogs.unwrap();

        save(false, &[("a", false, 0), ("b", true, 2), ("c", true, 1)]);
        assert_eq!(store.follow_nuvio_hero(&nuvio).unwrap(), vec![1]);
        assert_eq!(chosen(), ["c", "b"]);
        assert!(store.profile_settings(1).unwrap().home.hero_enabled, "the switch here is kept on the first look");

        store.update_profile_settings(1, |s| s.home.hero_catalogs = Some(vec!["a".into()])).unwrap();
        save(false, &[("a", false, 9), ("b", true, 0), ("c", true, 5)]);
        assert!(store.follow_nuvio_hero(&nuvio).unwrap().is_empty(), "only moved in Nuvio");
        assert_eq!(chosen(), ["a"]);

        save(true, &[("a", true, 0), ("b", true, 1), ("c", false, 2)]);
        assert_eq!(store.follow_nuvio_hero(&nuvio).unwrap(), vec![1]);
        assert_eq!(chosen(), ["a", "b"]);

        store.set_linked_user(Some("someone".into())).unwrap();
        save(true, &[("c", true, 0)]);
        assert!(store.follow_nuvio_hero(&nuvio).unwrap().is_empty(), "another account's data");
    }
}
