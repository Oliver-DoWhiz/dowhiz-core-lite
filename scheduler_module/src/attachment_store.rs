use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::AttachmentUploadRef;

#[derive(Debug, Clone)]
pub struct AttachmentUploadStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAttachmentMetadata {
    upload_id: String,
    file_name: String,
    content_type: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct MaterializedAttachment {
    upload_id: String,
    file_name: String,
    content_type: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct AttachmentManifest {
    attachment_names: Vec<String>,
    attachments: Vec<MaterializedAttachment>,
}

impl AttachmentUploadStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn stage_bytes(
        &self,
        file_name: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<AttachmentUploadRef> {
        let upload_id = Uuid::new_v4().to_string();
        let upload_dir = self.root.join(&upload_id);
        fs::create_dir_all(&upload_dir)?;

        let sanitized_name = sanitized_or_fallback(file_name, &upload_id);
        let metadata = StoredAttachmentMetadata {
            upload_id: upload_id.clone(),
            file_name: sanitized_name.clone(),
            content_type: content_type.trim().to_string(),
            size_bytes: bytes.len() as u64,
        };

        fs::write(upload_dir.join("payload.bin"), bytes)?;
        fs::write(
            upload_dir.join("metadata.json"),
            serde_json::to_string_pretty(&metadata)?,
        )?;

        Ok(AttachmentUploadRef {
            upload_id,
            file_name: sanitized_name,
            content_type: metadata.content_type,
            size_bytes: metadata.size_bytes,
        })
    }

    pub fn materialize_refs(
        &self,
        workspace_dir: &Path,
        refs: &[AttachmentUploadRef],
    ) -> Result<()> {
        let attachments_dir = workspace_dir.join("incoming_attachments");
        fs::create_dir_all(&attachments_dir)?;

        let mut attachment_names = Vec::new();
        let mut attachments = Vec::new();

        for reference in refs {
            let upload_dir = self.root.join(&reference.upload_id);
            let metadata = read_metadata(&upload_dir)?;
            let payload_path = upload_dir.join("payload.bin");
            let actual_size = fs::metadata(&payload_path)?.len();

            if actual_size != metadata.size_bytes {
                anyhow::bail!(
                    "staged attachment {} size mismatch: expected {}, got {}",
                    metadata.upload_id,
                    metadata.size_bytes,
                    actual_size
                );
            }

            let final_name = unique_file_name(&attachments_dir, &metadata.file_name);
            fs::copy(&payload_path, attachments_dir.join(&final_name))?;
            fs::remove_dir_all(&upload_dir)?;

            attachment_names.push(final_name.clone());
            attachments.push(MaterializedAttachment {
                upload_id: metadata.upload_id,
                file_name: final_name,
                content_type: metadata.content_type,
                size_bytes: metadata.size_bytes,
            });
        }

        fs::write(
            attachments_dir.join("thread_manifest.json"),
            serde_json::to_string_pretty(&AttachmentManifest {
                attachment_names,
                attachments,
            })?,
        )?;

        Ok(())
    }
}

fn read_metadata(upload_dir: &Path) -> Result<StoredAttachmentMetadata> {
    let metadata_path = upload_dir.join("metadata.json");
    let payload = fs::read_to_string(&metadata_path)
        .with_context(|| format!("failed to read {}", metadata_path.display()))?;
    serde_json::from_str(&payload)
        .with_context(|| format!("failed to parse {}", metadata_path.display()))
}

fn sanitized_or_fallback(file_name: &str, upload_id: &str) -> String {
    let sanitized = sanitize_file_name(file_name);
    if sanitized.is_empty() {
        format!("upload-{}.bin", &upload_id[..8])
    } else {
        sanitized
    }
}

fn sanitize_file_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn unique_file_name(dir: &Path, file_name: &str) -> String {
    if !dir.join(file_name).exists() {
        return file_name.to_string();
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("attachment");
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");

    for index in 2.. {
        let candidate = if extension.is_empty() {
            format!("{}_v{}", stem, index)
        } else {
            format!("{}_v{}.{}", stem, index, extension)
        };

        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }

    unreachable!("attachment name dedupe loop should always find a suffix");
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn stages_and_materializes_upload_refs() {
        let root = temp_dir("attachment-store");
        let store = AttachmentUploadStore::new(root.join("uploads")).unwrap();
        let workspace_dir = root.join("workspace");

        let first = store
            .stage_bytes("notes.txt", "text/plain", b"hello world")
            .unwrap();
        let second = store
            .stage_bytes("notes.txt", "text/plain", b"follow up")
            .unwrap();

        store
            .materialize_refs(&workspace_dir, &[first, second])
            .unwrap();

        assert_eq!(
            fs::read_to_string(workspace_dir.join("incoming_attachments/notes.txt")).unwrap(),
            "hello world"
        );
        assert_eq!(
            fs::read_to_string(workspace_dir.join("incoming_attachments/notes_v2.txt")).unwrap(),
            "follow up"
        );

        let manifest = fs::read_to_string(
            workspace_dir.join("incoming_attachments/thread_manifest.json"),
        )
        .unwrap();
        assert!(manifest.contains("\"notes.txt\""));
        assert!(manifest.contains("\"notes_v2.txt\""));
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{}-{}", prefix, unique));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
