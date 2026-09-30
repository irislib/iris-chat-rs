use super::*;
use crate::core::chat_pin_sync::ChatPinState;

impl AppStore {
    pub(crate) fn load_chat_pin_states(&self) -> anyhow::Result<BTreeMap<String, ChatPinState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut statement =
            conn.prepare("SELECT value FROM app_meta WHERE key LIKE 'chat_pin_state:%'")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut states = BTreeMap::new();
        for row in rows {
            let state: ChatPinState = serde_json::from_str(&row?)?;
            states.insert(state.chat_id.clone(), state);
        }
        Ok(states)
    }

    pub(crate) fn save_chat_pin_state(&mut self, state: &ChatPinState) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("chat_pin_state:{}", state.chat_id), serde_json::to_string(state)?],
        )?;
        Ok(())
    }
}
