use super::*;
use crate::core::chat_mute_sync::ChatMuteState;

impl AppStore {
    pub(crate) fn load_chat_mute_states(&self) -> anyhow::Result<BTreeMap<String, ChatMuteState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut statement =
            conn.prepare("SELECT value FROM app_meta WHERE key LIKE 'chat_mute_state:%'")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut states = BTreeMap::new();
        for row in rows {
            let state: ChatMuteState = serde_json::from_str(&row?)?;
            states.insert(state.chat_id.clone(), state);
        }
        Ok(states)
    }

    pub(crate) fn save_chat_mute_state(&mut self, state: &ChatMuteState) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("chat_mute_state:{}", state.chat_id), serde_json::to_string(state)?],
        )?;
        Ok(())
    }
}
