use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub(super) fn prepare(
    directory: PathBuf,
    attachments: Vec<OutgoingAttachment>,
    mut record: Record,
    keys: Keys,
) -> Result<Record, String> {
    let result = (|| {
        std::fs::create_dir_all(directory.parent().ok_or("Invalid folder")?)
            .map_err(|e| e.to_string())?;
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        for (index, input) in attachments.into_iter().enumerate() {
            let filename = input.filename.trim().to_string();
            if filename.is_empty()
                || filename.len() > 240
                || filename == "."
                || filename == ".."
                || filename
                    .chars()
                    .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
            {
                return Err("Choose a file with a valid name.".into());
            }
            let mut source = std::fs::File::open(&input.file_path)
                .map_err(|_| "The selected file could not be opened.")?;
            let meta = source.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                return Err("The selected file is too large or is not a regular file.".into());
            }
            // Keep the extension so desktop Open actions retain file associations.
            let path = directory.join(format!("{index}-{filename}"));
            let mut target = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| e.to_string())?;
            let mut digest = Sha256::new();
            let mut size = 0u64;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = source.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > MAX_FILE_BYTES {
                    return Err("The selected file is too large.".into());
                }
                let bytes = buf.get(..n).ok_or("Could not read the selected file.")?;
                target.write_all(bytes).map_err(|e| e.to_string())?;
                digest.update(bytes);
            }
            target.sync_all().map_err(|e| e.to_string())?;
            record.paths.push(path.to_string_lossy().into_owned());
            record.offer.files.push(ManifestFile {
                filename,
                size_bytes: size,
                sha256: format!("{:x}", digest.finalize()),
            });
        }
        record.offer.expires_at_secs = unix_now().get().saturating_add(24 * 60 * 60);
        record.wire = record.offer.wire(&keys)?;
        Ok(record)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result
}
