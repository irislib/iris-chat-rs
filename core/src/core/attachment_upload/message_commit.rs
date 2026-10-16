use super::*;

fn roots(store: &AttachmentChunkStore, text: &str) -> anyhow::Result<Vec<Hash>> {
    // Bound the result by the durable staging index, not by the number of
    // caption links. Repeated or existing links can precede every new upload.
    let mut staged: HashSet<Hash> = store
        .staged_references()?
        .into_iter()
        .map(|(root, _, _)| root)
        .collect();
    if staged.is_empty() {
        return Ok(Vec::new());
    }
    Ok(extract_message_attachments(text)
        .1
        .into_iter()
        .filter_map(|attachment| nhash_decode(&attachment.nhash).ok().map(|data| data.hash))
        .filter(|root| staged.remove(root))
        .collect())
}

impl AppCore {
    /// Record the authored identity before any message persistence or publish.
    /// Neither the body, filename nor decryption key enters blob metadata.
    pub(in crate::core) fn bind_attachment_message(
        &mut self,
        chat: &str,
        message: &str,
        text: &str,
    ) -> anyhow::Result<bool> {
        let Some(service) = &self.attachment_service else {
            return Ok(false);
        };
        service
            .store
            .bind_message(&roots(&service.store, text)?, chat, message)
    }

    pub(in crate::core) fn resolve_attachment_staging(
        &self,
        store: &AttachmentChunkStore,
        selected: Option<&[Hash]>,
    ) -> anyhow::Result<()> {
        for (root, chat, message) in store.staged_references()? {
            if selected.is_some_and(|selected| !selected.contains(&root)) {
                continue;
            }
            let durable = match (chat, message) {
                (Some(chat), Some(message)) => {
                    self.app_store.message_exists(&chat, Some(&message), None)?
                }
                _ => false,
            };
            store.resolve_staged(&root, durable)?;
        }
        Ok(())
    }

    pub(in crate::core) fn finish_attachment_message(&mut self, text: &str) {
        if let Some(service) = &self.attachment_service {
            let result = roots(&service.store, text)
                .and_then(|roots| self.resolve_attachment_staging(&service.store, Some(&roots)));
            if let Err(error) = result {
                // Unknown private-store state is not proof that a file is unsent.
                // Keep its durable pin; startup retries the same reconciliation.
                self.push_debug_log("attachment.message.commit.error", error.to_string());
            }
        }
    }
}

#[cfg(test)]
#[path = "message_commit_tests.rs"]
mod tests;
