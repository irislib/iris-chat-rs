use std::collections::HashMap;

use iris_chat_core::OutgoingAttachment;
use tempfile::NamedTempFile;

#[derive(Default)]
pub(super) struct AttachmentDrafts {
    chats: HashMap<String, Vec<OutgoingAttachment>>,
    generation: u64,
    images: HashMap<String, NamedTempFile>,
    // Core ingestion is asynchronous. Keep only the generated source files (no
    // pixel buffers) through the app session, including direct-send preparation.
    // NamedTempFile removes them when the manager is dropped on app shutdown.
    submitted_images: Vec<NamedTempFile>,
}

impl AttachmentDrafts {
    pub fn for_application() -> std::rc::Rc<std::cell::RefCell<Self>> {
        use gtk::gio::prelude::ApplicationExt;
        let drafts = std::rc::Rc::new(std::cell::RefCell::new(Self::default()));
        if let Some(application) = gtk::gio::Application::default() {
            let drafts = drafts.clone();
            application.connect_shutdown(move |_| *drafts.borrow_mut() = Self::default());
        }
        drafts
    }

    pub fn get(&self, chat: &str) -> Vec<OutgoingAttachment> {
        self.chats.get(chat).cloned().unwrap_or_default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn stage(&mut self, chat: &str, attachment: OutgoingAttachment) {
        let files = self.chats.entry(chat.into()).or_default();
        if !files
            .iter()
            .any(|file| file.file_path == attachment.file_path)
        {
            files.push(attachment);
        }
    }

    pub fn stage_image(&mut self, chat: &str, image: NamedTempFile) {
        let path = image.path().to_string_lossy().into_owned();
        self.stage(
            chat,
            OutgoingAttachment {
                filename: "Pasted image.png".into(),
                file_path: path.clone(),
            },
        );
        self.images.insert(path, image);
    }

    pub fn remove(&mut self, chat: &str, path: &str) {
        if let Some(files) = self.chats.get_mut(chat) {
            files.retain(|file| file.file_path != path);
        }
        self.images.remove(path); // Original copied files are never owned here.
    }

    pub fn take(&mut self, chat: &str) -> Vec<OutgoingAttachment> {
        // Even a text-only Send ends this draft. An asynchronous clipboard read
        // started before it must not add files to the next message.
        self.generation = self.generation.wrapping_add(1);
        let files = self.chats.remove(chat).unwrap_or_default();
        for file in &files {
            if let Some(image) = self.images.remove(&file.file_path) {
                self.submitted_images.push(image);
            }
        }
        files
    }

    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.chats.clear();
        self.images.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_clear_end_the_draft_even_without_attachments() {
        let mut drafts = AttachmentDrafts::default();
        let before_send = drafts.generation();
        assert!(drafts.take("text-only-chat").is_empty());
        assert_ne!(drafts.generation(), before_send);
        let before_clear = drafts.generation();
        drafts.clear();
        assert_ne!(drafts.generation(), before_clear);
    }

    #[test]
    fn owns_only_generated_images_and_keeps_submitted_sources_alive() {
        let original = NamedTempFile::new().unwrap();
        let image = NamedTempFile::new().unwrap();
        let image_path = image.path().to_owned();
        let mut drafts = AttachmentDrafts::default();
        drafts.stage(
            "a",
            OutgoingAttachment {
                filename: "original".into(),
                file_path: original.path().to_string_lossy().into(),
            },
        );
        drafts.stage_image("a", image);
        drafts.remove("a", original.path().to_str().unwrap());
        assert!(original.path().exists());
        assert_eq!(drafts.take("a").len(), 1);
        drafts.clear();
        assert!(
            image_path.exists(),
            "queued core ingestion still needs the source"
        );
        drop(drafts);
        assert!(!image_path.exists());

        let image = NamedTempFile::new().unwrap();
        let image_path = image.path().to_owned();
        let mut drafts = AttachmentDrafts::default();
        drafts.stage_image("a", image);
        drafts.remove("a", image_path.to_str().unwrap());
        assert!(
            !image_path.exists(),
            "removing an unsent screenshot cleans it up"
        );
    }
}
