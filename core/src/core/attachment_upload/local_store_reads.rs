use super::*;
use hashtree_core::{decode_tree_node_by_cid, encrypted_size_chk, LinkType};

#[derive(Clone)]
pub(super) struct ReadReference {
    cid: Cid,
    link: Option<(LinkType, u64)>,
}

impl ReadReference {
    fn expected_blob_size(&self) -> Option<u64> {
        self.link.map(|(_, size)| {
            if self.cid.key.is_some() {
                encrypted_size_chk(size as usize) as u64
            } else {
                size
            }
        })
    }
}

impl AttachmentChunkStore {
    pub(in crate::core::attachment_upload) fn reading_tree(
        self: &Arc<Self>,
        cid: &Cid,
    ) -> anyhow::Result<ReadLease> {
        let lease = self.reading()?;
        lease.protect(
            ReadReference {
                cid: cid.clone(),
                link: None,
            },
            None,
        )?;
        Ok(lease)
    }
}

impl ReadLease {
    /// Expand under the same store mutex used by writes/eviction. A cached
    /// manifest and all its available descendants therefore become protected
    /// together, before an assembling reader can yield to another writer.
    fn protect(&self, root: ReadReference, supplied: Option<&[u8]>) -> anyhow::Result<()> {
        let mut references = self
            .references
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment read mutex poisoned"))?;
        let mut state = self.store.lock()?;
        let initial = root.cid.hash;
        let mut queue = vec![root];
        let mut visited = HashSet::new();
        while let Some(reference) = queue.pop() {
            let hash = reference.cid.hash;
            if !visited.insert(hash) {
                continue;
            }
            anyhow::ensure!(
                visited.len() <= MAX_TREE_BLOCKS,
                "attachment tree has too many blocks"
            );
            let reference_count = references.len();
            if let std::collections::hash_map::Entry::Vacant(entry) = references.entry(hash) {
                anyhow::ensure!(
                    reference_count < MAX_TREE_BLOCKS,
                    "attachment tree has too many blocks"
                );
                entry.insert(reference.clone());
                *state.pins.entry(hash).or_default() += 1;
                self.store.sync_pin(&state, &hash)?;
            }
            let expected_size = reference.expected_blob_size();
            // Blob links with matching lengths are leaves in Hashtree's reader.
            // Their hashes are pinned without loading/decrypting every file chunk.
            if matches!(reference.link, Some((LinkType::Blob, _)))
                && state.blobs()?.blob_size_sync(&hash)? == expected_size
            {
                continue;
            }
            let bytes = match supplied.filter(|_| hash == initial) {
                Some(bytes) => Some(bytes.to_vec()),
                // An unreadable cached manifest can still be repaired by the
                // normal hash-verifying HTTP/peer read. It stays pinned meanwhile.
                None => self.store.read_locked(&state, &hash).ok().flatten(),
            };
            let Some(bytes) = bytes else {
                continue;
            };
            anyhow::ensure!(
                hashtree_core::sha256(&bytes) == hash,
                "corrupt attachment block"
            );
            let length = bytes.len() as u64;
            let node = match decode_tree_node_by_cid(&reference.cid, bytes) {
                Ok(node) => node,
                Err(_)
                    if matches!(reference.link, Some((LinkType::File, _)))
                        && expected_size == Some(length) =>
                {
                    None
                }
                Err(error) => return Err(error.into()),
            };
            let Some(node) = node else {
                continue;
            };
            if matches!(reference.link, Some((LinkType::Blob, _)))
                && node.node_type != LinkType::File
            {
                continue;
            }
            anyhow::ensure!(
                node.links.len() <= MAX_TREE_BLOCKS
                    && queue.len() + node.links.len() <= MAX_TREE_BLOCKS,
                "attachment tree has too many links"
            );
            for link in node.links {
                anyhow::ensure!(
                    link.size <= MAX_ATTACHMENT_BYTES,
                    "attachment link is too large"
                );
                queue.push(ReadReference {
                    cid: link.to_cid(),
                    link: Some((link.link_type, link.size)),
                });
            }
        }
        Ok(())
    }

    /// A cold fetched manifest reveals child references. Keep their keys only
    /// in this bounded read lease, and protect later cache admission by hash.
    pub(in crate::core::attachment_upload) fn observe(
        &self,
        hash: &Hash,
        bytes: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_ATTACHMENT_BYTES + 64,
            "attachment block is too large"
        );
        anyhow::ensure!(
            hashtree_core::sha256(bytes) == *hash,
            "corrupt attachment block"
        );
        let reference = self
            .references
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment read mutex poisoned"))?
            .get(hash)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unexpected attachment block"))?;
        self.protect(reference, Some(bytes))
    }
}
