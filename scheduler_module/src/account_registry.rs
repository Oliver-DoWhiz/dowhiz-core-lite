use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{CreateTaskRequest, InboundTaskRequest};

#[derive(Debug)]
pub struct AccountRegistry {
    path: PathBuf,
    memory_root: PathBuf,
    data: Mutex<AccountRegistryData>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountRegistryData {
    #[serde(default)]
    pub identifiers_by_account_id: HashMap<String, AccountIdentifiers>,
    #[serde(default)]
    pub memory_path_by_account_id: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountIdentifiers {
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub slack_user_ids: Vec<String>,
    #[serde(default)]
    pub discord_user_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccount {
    pub account_id: Option<String>,
    pub memory_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum AccountRegistryError {
    AccountIdTaken(String),
    EmailAlreadyBound { email: String, account_id: String },
    InvalidAccountId(String),
    Storage(anyhow::Error),
}

impl std::fmt::Display for AccountRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccountIdTaken(account_id) => {
                write!(f, "account_id '{}' has already been taken", account_id)
            }
            Self::EmailAlreadyBound { email, account_id } => {
                write!(f, "email '{}' is already linked to account '{}'", email, account_id)
            }
            Self::InvalidAccountId(account_id) => write!(
                f,
                "account_id '{}' must use only letters, numbers, hyphens, or underscores",
                account_id
            ),
            Self::Storage(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for AccountRegistryError {}

impl From<anyhow::Error> for AccountRegistryError {
    fn from(err: anyhow::Error) -> Self {
        Self::Storage(err)
    }
}

impl From<std::io::Error> for AccountRegistryError {
    fn from(err: std::io::Error) -> Self {
        Self::Storage(err.into())
    }
}

impl From<serde_json::Error> for AccountRegistryError {
    fn from(err: serde_json::Error) -> Self {
        Self::Storage(err.into())
    }
}

impl AccountRegistry {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let memory_root = default_memory_root(&path);
        if !path.exists() {
            return Ok(Self {
                path,
                memory_root,
                data: Mutex::new(AccountRegistryData::default()),
            });
        }

        let payload = fs::read_to_string(&path)
            .with_context(|| format!("failed to read account registry {}", path.display()))?;
        let data = serde_json::from_str::<AccountRegistryData>(&payload).with_context(|| {
            format!("failed to parse account registry {}", path.display())
        })?;

        Ok(Self {
            path,
            memory_root,
            data: Mutex::new(data),
        })
    }

    pub fn resolve_create_request(
        &self,
        request: CreateTaskRequest,
    ) -> std::result::Result<(InboundTaskRequest, ResolvedAccount), AccountRegistryError> {
        let reply_to = if request.reply_to.trim().is_empty() {
            request.customer_email.trim().to_string()
        } else {
            request.reply_to.trim().to_string()
        };

        let internal = InboundTaskRequest {
            customer_email: request.customer_email.trim().to_string(),
            subject: request.subject.trim().to_string(),
            prompt: request.prompt,
            channel: request.channel,
            reply_to,
            tenant_id: request.tenant_id.trim().to_string(),
            account_id: request.account_id.trim().to_string(),
            memory_uri: String::new(),
            identity_uri: String::new(),
            credential_refs: Vec::new(),
        };

        self.resolve_request(internal, request.register_account_id)
    }

    pub fn resolve_inbound_request(
        &self,
        request: InboundTaskRequest,
    ) -> std::result::Result<(InboundTaskRequest, ResolvedAccount), AccountRegistryError> {
        self.resolve_request(request, false)
    }

    pub fn generate_available_account_id(
        &self,
    ) -> std::result::Result<String, AccountRegistryError> {
        let data = self.lock_data()?;
        for _ in 0..32 {
            let suffix = Uuid::new_v4().simple().to_string();
            let candidate = format!("acct_{}", &suffix[..12]);
            if !data.identifiers_by_account_id.contains_key(&candidate) {
                return Ok(candidate);
            }
        }

        Err(anyhow!("failed to generate a unique account_id").into())
    }

    pub fn materialize_memory(&self, workspace_dir: &Path, resolved: &ResolvedAccount) -> Result<()> {
        let Some(memory_path) = resolved.memory_path.as_ref() else {
            return Ok(());
        };

        ensure_memory_source(memory_path)?;

        let target_dir = workspace_dir.join("memory");
        copy_memory_tree(memory_path, &target_dir)
    }

    fn resolve_request(
        &self,
        mut request: InboundTaskRequest,
        register_account_id: bool,
    ) -> std::result::Result<(InboundTaskRequest, ResolvedAccount), AccountRegistryError> {
        let resolved = self.resolve_account(
            &request.customer_email,
            &request.account_id,
            register_account_id,
        )?;

        if let Some(account_id) = resolved.account_id.as_deref() {
            request.account_id = account_id.to_string();
            request.identity_uri = format!("account_registry://{}", account_id);
        }

        if let Some(memory_path) = resolved.memory_path.as_ref() {
            request.memory_uri = memory_path.display().to_string();
        }

        Ok((request, resolved))
    }

    fn resolve_account(
        &self,
        customer_email: &str,
        requested_account_id: &str,
        register_account_id: bool,
    ) -> std::result::Result<ResolvedAccount, AccountRegistryError> {
        let requested_account_id = requested_account_id.trim();
        if !requested_account_id.is_empty() {
            validate_account_id(requested_account_id)?;

            let mut data = self.lock_data()?;
            if register_account_id && data.identifiers_by_account_id.contains_key(requested_account_id) {
                return Err(AccountRegistryError::AccountIdTaken(
                    requested_account_id.to_string(),
                ));
            }

            ensure_email_not_bound_elsewhere(&data, requested_account_id, customer_email)?;

            let mut dirty = false;
            let identifiers = data
                .identifiers_by_account_id
                .entry(requested_account_id.to_string())
                .or_default();
            if append_email(&mut identifiers.emails, customer_email) {
                dirty = true;
            }

            let (memory_path, memory_dirty) =
                self.ensure_memory_path_locked(&mut data, requested_account_id)?;
            dirty |= memory_dirty;

            if dirty {
                self.persist_locked(&data)?;
            }

            return Ok(ResolvedAccount {
                account_id: Some(requested_account_id.to_string()),
                memory_path: Some(memory_path),
            });
        }

        let normalized_email = normalize_identifier(customer_email);
        if normalized_email.is_empty() {
            return Ok(ResolvedAccount {
                account_id: None,
                memory_path: None,
            });
        }

        let mut data = self.lock_data()?;
        let Some((account_id, _)) = data
            .identifiers_by_account_id
            .iter()
            .find(|(_, identifiers)| {
                identifiers
                    .emails
                    .iter()
                    .any(|email| normalize_identifier(email) == normalized_email)
            })
        else {
            return Ok(ResolvedAccount {
                account_id: None,
                memory_path: None,
            });
        };
        let account_id = account_id.clone();
        let (memory_path, dirty) = self.ensure_memory_path_locked(&mut data, &account_id)?;
        if dirty {
            self.persist_locked(&data)?;
        }

        Ok(ResolvedAccount {
            account_id: Some(account_id),
            memory_path: Some(memory_path),
        })
    }

    fn ensure_memory_path_locked(
        &self,
        data: &mut AccountRegistryData,
        account_id: &str,
    ) -> std::result::Result<(PathBuf, bool), AccountRegistryError> {
        if let Some(memory_path) = data
            .memory_path_by_account_id
            .get(account_id)
            .map(|value| PathBuf::from(value.trim()))
            .filter(|value| !value.as_os_str().is_empty())
        {
            ensure_memory_source(&memory_path)?;
            return Ok((memory_path, false));
        }

        let memory_path = self.memory_root.join(sanitize_account_id_segment(account_id));
        ensure_memory_source(&memory_path)?;
        data.memory_path_by_account_id.insert(
            account_id.to_string(),
            memory_path.display().to_string(),
        );
        Ok((memory_path, true))
    }

    fn persist_locked(
        &self,
        data: &AccountRegistryData,
    ) -> std::result::Result<(), AccountRegistryError> {
        let payload = serde_json::to_string_pretty(data)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let tmp_path = self.path.with_extension("tmp");
        fs::write(&tmp_path, payload)
            .with_context(|| format!("failed to write account registry {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &self.path).with_context(|| {
            format!(
                "failed to move account registry temp file into place: {}",
                self.path.display()
            )
        })?;
        Ok(())
    }

    fn lock_data(
        &self,
    ) -> std::result::Result<MutexGuard<'_, AccountRegistryData>, AccountRegistryError> {
        self.data
            .lock()
            .map_err(|_| anyhow!("account registry lock poisoned").into())
    }
}

pub fn persist_workspace_memory(workspace_dir: &Path, memory_uri: &str) -> Result<()> {
    let memory_uri = memory_uri.trim();
    if memory_uri.is_empty() {
        return Ok(());
    }

    let source_dir = workspace_dir.join("memory");
    if !source_dir.exists() {
        return Ok(());
    }

    let target = PathBuf::from(memory_uri);
    if looks_like_file_path(&target) {
        let file_name = target
            .file_name()
            .context("configured memory file path does not have a file name")?;
        let source_file = source_dir.join(file_name);
        let fallback = source_dir.join("memo.md");
        let source_file = if source_file.exists() {
            source_file
        } else if fallback.exists() {
            fallback
        } else {
            return Ok(());
        };

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source_file, target)?;
        return Ok(());
    }

    remove_existing_path(&target)?;
    copy_memory_tree(&source_dir, &target)
}

fn copy_memory_tree(source: &Path, target: &Path) -> Result<()> {
    if source.is_file() {
        fs::create_dir_all(target)?;
        let file_name = source
            .file_name()
            .context("memory file path does not have a file name")?;
        fs::copy(source, target.join(file_name))?;
        return Ok(());
    }

    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        let entry_path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_memory_tree(&entry_path, &destination)?;
        } else {
            fs::copy(entry_path, destination)?;
        }
    }

    Ok(())
}

fn default_memory_root(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("account_memories")
}

fn ensure_memory_source(path: &Path) -> Result<()> {
    if looks_like_file_path(path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(path, blank_memo_contents())?;
        }
        return Ok(());
    }

    fs::create_dir_all(path)?;
    let memo_path = path.join("memo.md");
    if !memo_path.exists() {
        fs::write(memo_path, blank_memo_contents())?;
    }
    Ok(())
}

fn blank_memo_contents() -> &'static str {
    "# Memo\n"
}

fn looks_like_file_path(path: &Path) -> bool {
    path.extension().is_some()
}

fn remove_existing_path(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn validate_account_id(account_id: &str) -> std::result::Result<(), AccountRegistryError> {
    if account_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        Ok(())
    } else {
        Err(AccountRegistryError::InvalidAccountId(account_id.to_string()))
    }
}

fn sanitize_account_id_segment(account_id: &str) -> String {
    let sanitized = account_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "anonymous".to_string()
    } else {
        sanitized
    }
}

fn append_email(emails: &mut Vec<String>, email: &str) -> bool {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return false;
    }

    let normalized = normalize_identifier(trimmed);
    if emails
        .iter()
        .any(|existing| normalize_identifier(existing) == normalized)
    {
        return false;
    }

    emails.push(trimmed.to_string());
    true
}

fn ensure_email_not_bound_elsewhere(
    data: &AccountRegistryData,
    target_account_id: &str,
    customer_email: &str,
) -> std::result::Result<(), AccountRegistryError> {
    let normalized_email = normalize_identifier(customer_email);
    if normalized_email.is_empty() {
        return Ok(());
    }

    for (account_id, identifiers) in &data.identifiers_by_account_id {
        if account_id == target_account_id {
            continue;
        }

        if identifiers
            .emails
            .iter()
            .any(|email| normalize_identifier(email) == normalized_email)
        {
            return Err(AccountRegistryError::EmailAlreadyBound {
                email: customer_email.trim().to_string(),
                account_id: account_id.clone(),
            });
        }
    }

    Ok(())
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::models::CreateTaskRequest;

    #[test]
    fn resolves_account_by_email_and_injects_memory_path() {
        let root = temp_dir("account-registry");
        let memory_dir = root.join("memory-source");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("memo.md"), "# hello").unwrap();

        let registry_path = root.join("account_registry.json");
        fs::write(
            &registry_path,
            serde_json::to_string_pretty(&AccountRegistryData {
                identifiers_by_account_id: HashMap::from([(
                    "acct_123".to_string(),
                    AccountIdentifiers {
                        emails: vec!["dtang04@uchicago.edu".to_string()],
                        phones: vec!["+16309153426".to_string()],
                        slack_user_ids: Vec::new(),
                        discord_user_ids: Vec::new(),
                    },
                )]),
                memory_path_by_account_id: HashMap::from([(
                    "acct_123".to_string(),
                    memory_dir.display().to_string(),
                )]),
            })
            .unwrap(),
        )
        .unwrap();

        let registry = AccountRegistry::load(&registry_path).unwrap();
        let (request, resolved) = registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: String::new(),
                tenant_id: String::new(),
                account_id: String::new(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();

        assert_eq!(request.account_id, "acct_123");
        assert_eq!(request.reply_to, "dtang04@uchicago.edu");
        assert_eq!(
            request.identity_uri,
            "account_registry://acct_123".to_string()
        );
        assert_eq!(
            resolved.memory_path,
            Some(PathBuf::from(memory_dir.display().to_string()))
        );

        let workspace_dir = root.join("workspace");
        registry.materialize_memory(&workspace_dir, &resolved).unwrap();
        assert_eq!(
            fs::read_to_string(workspace_dir.join("memory/memo.md")).unwrap(),
            "# hello"
        );
    }

    #[test]
    fn creates_missing_account_and_blank_memory_for_explicit_account_id() {
        let root = temp_dir("account-registry-create");
        let registry_path = root.join("account_registry.json");
        let registry = AccountRegistry::load(&registry_path).unwrap();
        let (request, resolved) = registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: "reply@example.com".to_string(),
                tenant_id: String::new(),
                account_id: "acct_manual".to_string(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();

        assert_eq!(request.account_id, "acct_manual");
        assert_eq!(
            resolved.account_id,
            Some("acct_manual".to_string())
        );
        let memory_path = resolved.memory_path.unwrap();
        assert_eq!(memory_path, root.join("account_memories").join("acct_manual"));
        assert_eq!(fs::read_to_string(memory_path.join("memo.md")).unwrap(), "# Memo\n");

        let persisted: AccountRegistryData =
            serde_json::from_str(&fs::read_to_string(registry_path).unwrap()).unwrap();
        assert_eq!(
            persisted
                .identifiers_by_account_id
                .get("acct_manual")
                .unwrap()
                .emails,
            vec!["dtang04@uchicago.edu".to_string()]
        );
    }

    #[test]
    fn rejects_registering_an_already_taken_account_id() {
        let root = temp_dir("account-registry-taken");
        let registry_path = root.join("account_registry.json");
        fs::write(
            &registry_path,
            serde_json::to_string_pretty(&AccountRegistryData {
                identifiers_by_account_id: HashMap::from([(
                    "acct_manual".to_string(),
                    AccountIdentifiers::default(),
                )]),
                memory_path_by_account_id: HashMap::new(),
            })
            .unwrap(),
        )
        .unwrap();

        let registry = AccountRegistry::load(&registry_path).unwrap();
        let err = registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: "reply@example.com".to_string(),
                tenant_id: String::new(),
                account_id: "acct_manual".to_string(),
                register_account_id: true,
                attachment_refs: Vec::new(),
            })
            .unwrap_err();

        assert!(matches!(err, AccountRegistryError::AccountIdTaken(ref value) if value == "acct_manual"));
    }

    #[test]
    fn appends_new_email_to_existing_account() {
        let root = temp_dir("account-registry-email");
        let registry_path = root.join("account_registry.json");
        fs::write(
            &registry_path,
            serde_json::to_string_pretty(&AccountRegistryData {
                identifiers_by_account_id: HashMap::from([(
                    "acct_manual".to_string(),
                    AccountIdentifiers {
                        emails: vec!["dylan@dowhiz.com".to_string()],
                        phones: Vec::new(),
                        slack_user_ids: Vec::new(),
                        discord_user_ids: Vec::new(),
                    },
                )]),
                memory_path_by_account_id: HashMap::new(),
            })
            .unwrap(),
        )
        .unwrap();

        let registry = AccountRegistry::load(&registry_path).unwrap();
        registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: "reply@example.com".to_string(),
                tenant_id: String::new(),
                account_id: "acct_manual".to_string(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();

        let persisted: AccountRegistryData =
            serde_json::from_str(&fs::read_to_string(registry_path).unwrap()).unwrap();
        assert_eq!(
            persisted
                .identifiers_by_account_id
                .get("acct_manual")
                .unwrap()
                .emails,
            vec![
                "dylan@dowhiz.com".to_string(),
                "dtang04@uchicago.edu".to_string()
            ]
        );
    }

    #[test]
    fn initializes_missing_memo_for_existing_memory_path_without_rewriting_registry() {
        let root = temp_dir("account-registry-init-existing-memory");
        let memory_dir = root.join("memory-source");
        fs::create_dir_all(&memory_dir).unwrap();

        let registry_path = root.join("account_registry.json");
        let registry_payload = serde_json::to_string_pretty(&AccountRegistryData {
            identifiers_by_account_id: HashMap::from([(
                "acct_123".to_string(),
                AccountIdentifiers {
                    emails: vec!["dtang04@uchicago.edu".to_string()],
                    phones: Vec::new(),
                    slack_user_ids: Vec::new(),
                    discord_user_ids: Vec::new(),
                },
            )]),
            memory_path_by_account_id: HashMap::from([(
                "acct_123".to_string(),
                memory_dir.display().to_string(),
            )]),
        })
        .unwrap();
        fs::write(&registry_path, &registry_payload).unwrap();

        let registry = AccountRegistry::load(&registry_path).unwrap();
        let (_, resolved) = registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: String::new(),
                tenant_id: String::new(),
                account_id: String::new(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();

        assert_eq!(resolved.account_id, Some("acct_123".to_string()));
        assert_eq!(resolved.memory_path, Some(memory_dir.clone()));
        assert_eq!(
            fs::read_to_string(memory_dir.join("memo.md")).unwrap(),
            "# Memo\n"
        );
        assert_eq!(fs::read_to_string(registry_path).unwrap(), registry_payload);
    }

    #[test]
    fn persists_workspace_memory_back_to_source_directory() {
        let root = temp_dir("account-registry-writeback");
        let workspace_dir = root.join("workspace");
        let source_dir = root.join("memory-source");
        fs::create_dir_all(workspace_dir.join("memory")).unwrap();
        fs::write(workspace_dir.join("memory/memo.md"), "# Updated").unwrap();
        fs::write(workspace_dir.join("memory/preferences.md"), "- likes tea").unwrap();
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("memo.md"), "# Old").unwrap();

        persist_workspace_memory(&workspace_dir, &source_dir.display().to_string()).unwrap();

        assert_eq!(fs::read_to_string(source_dir.join("memo.md")).unwrap(), "# Updated");
        assert_eq!(
            fs::read_to_string(source_dir.join("preferences.md")).unwrap(),
            "- likes tea"
        );
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
