use super::*;

type StagedReference = (Hash, Option<String>, Option<String>);

impl AttachmentChunkStore {
    pub(in crate::core) fn bind_message(
        &self,
        roots: &[Hash],
        chat: &str,
        message: &str,
    ) -> anyhow::Result<bool> {
        let mut state = self.lock()?;
        let tx = state.db.transaction()?;
        let mut bound = false;
        for root in roots {
            bound |= tx.execute(
                "UPDATE trees SET chat_id=?2,message_id=?3 WHERE root=?1 AND committed=0",
                params![to_hex(root), chat, message],
            )? != 0;
        }
        tx.commit()?;
        self.blobs.force_sync()?;
        Ok(bound)
    }

    pub(in crate::core) fn staged_references(&self) -> anyhow::Result<Vec<StagedReference>> {
        let state = self.lock()?;
        let mut stmt = state
            .db
            .prepare("SELECT root,chat_id,message_id FROM trees WHERE committed=0 LIMIT 257")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))?;
        let result = rows
            .map(|row| {
                let (hash, chat, message) = row?;
                Ok((hashtree_core::from_hex(&hash)?, chat, message))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(result.len() <= MAX_TREES, "attachment stage count exceeded");
        Ok(result)
    }

    pub(in crate::core) fn resolve_staged(&self, root: &Hash, durable: bool) -> anyhow::Result<()> {
        let state = self.lock()?;
        let hashes = {
            let mut stmt = state
                .db
                .prepare("SELECT hash FROM tree_blocks WHERE root=?1 LIMIT 129")?;
            let rows = stmt.query_map([to_hex(root)], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(hashtree_core::from_hex(&row?)?))
                .collect::<anyhow::Result<Vec<_>>>()?
        };
        anyhow::ensure!(hashes.len() <= MAX_TREE_BLOCKS, "invalid attachment stage");
        if durable {
            state.db.execute("UPDATE trees SET committed=1,chat_id=NULL,message_id=NULL WHERE root=?1 AND committed=0", [to_hex(root)])?;
        } else {
            state.db.execute(
                "DELETE FROM trees WHERE root=?1 AND committed=0",
                [to_hex(root)],
            )?;
        }
        for hash in hashes {
            self.sync_pin(&state, &hash)?;
        }
        self.blobs.force_sync()?;
        self.wake.notify_one();
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::core) fn raw_put_for_test(&self, hash: Hash, bytes: &[u8]) {
        self.blobs.delete_sync(&hash).unwrap();
        self.blobs.put_sync(hash, bytes).unwrap();
        self.blobs.force_sync().unwrap();
    }
}
