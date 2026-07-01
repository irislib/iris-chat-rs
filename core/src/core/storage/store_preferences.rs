use super::{
    hash_preferences, load_preferences, write_preferences, AppStore, PersistedPreferences,
};
use crate::state::PreferencesSnapshot;

impl AppStore {
    pub(crate) fn load_preferences_snapshot(
        &mut self,
    ) -> anyhow::Result<Option<PersistedPreferences>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_preferences(&conn)
    }

    pub(crate) fn save_preferences(
        &mut self,
        preferences: &PreferencesSnapshot,
    ) -> anyhow::Result<()> {
        let preferences_hash = hash_preferences(preferences);
        if self.cache.preferences == Some(preferences_hash) {
            return Ok(());
        }

        {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            write_preferences(&tx, preferences)?;
            tx.commit()?;
        }

        self.cache.preferences = Some(preferences_hash);
        Ok(())
    }
}
