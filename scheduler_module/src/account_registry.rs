use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::{CreateTaskRequest, InboundTaskRequest};

#[derive(Debug, Clone, Default)]
pub struct AccountRegistry {
    data: AccountRegistryData,
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

impl AccountRegistry {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self::default());
        }

        let payload = fs::read_to_string(&path)
            .with_context(|| format!("failed to read account registry {}", path.display()))?;
        let data = serde_json::from_str::<AccountRegistryData>(&payload).with_context(|| {
            format!("failed to parse account registry {}", path.display())
        })?;

        Ok(Self { data })
    }

    pub fn resolve_create_request(
        &self,
        request: CreateTaskRequest,
    ) -> (InboundTaskRequest, ResolvedAccount) {
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

        self.resolve_inbound_request(internal)
    }

    pub fn resolve_inbound_request(
        &self,
        mut request: InboundTaskRequest,
    ) -> (InboundTaskRequest, ResolvedAccount) {
        let resolved = self.resolve_account(&request.customer_email, &request.account_id);

        if let Some(account_id) = resolved.account_id.as_deref() {
            request.account_id = account_id.to_string();
            request.identity_uri = format!("account_registry://{}", account_id);
        }

        if let Some(memory_path) = resolved.memory_path.as_ref() {
            request.memory_uri = memory_path.display().to_string();
        }

        (request, resolved)
    }

    pub fn materialize_memory(&self, workspace_dir: &Path, resolved: &ResolvedAccount) -> Result<()> {
        let Some(memory_path) = resolved.memory_path.as_ref() else {
            return Ok(());
        };

        if !memory_path.exists() {
            anyhow::bail!(
                "configured memory path does not exist for account {:?}: {}",
                resolved.account_id,
                memory_path.display()
            );
        }

        let target_dir = workspace_dir.join("memory");
        copy_memory_tree(memory_path, &target_dir)
    }

    fn resolve_account(&self, customer_email: &str, requested_account_id: &str) -> ResolvedAccount {
        let requested_account_id = requested_account_id.trim();
        if !requested_account_id.is_empty() {
            return ResolvedAccount {
                account_id: Some(requested_account_id.to_string()),
                memory_path: self.memory_path_for_account(requested_account_id),
            };
        }

        let normalized_email = normalize_identifier(customer_email);
        if normalized_email.is_empty() {
            return ResolvedAccount {
                account_id: None,
                memory_path: None,
            };
        }

        let Some((account_id, _)) = self
            .data
            .identifiers_by_account_id
            .iter()
            .find(|(_, identifiers)| {
                identifiers
                    .emails
                    .iter()
                    .any(|email| normalize_identifier(email) == normalized_email)
            })
        else {
            return ResolvedAccount {
                account_id: None,
                memory_path: None,
            };
        };

        ResolvedAccount {
            account_id: Some(account_id.clone()),
            memory_path: self.memory_path_for_account(account_id),
        }
    }

    fn memory_path_for_account(&self, account_id: &str) -> Option<PathBuf> {
        self.data
            .memory_path_by_account_id
            .get(account_id)
            .map(|value| PathBuf::from(value.trim()))
            .filter(|value| !value.as_os_str().is_empty())
    }
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
        let (request, resolved) = registry.resolve_create_request(CreateTaskRequest {
            customer_email: "dtang04@uchicago.edu".to_string(),
            subject: "Need help".to_string(),
            prompt: "Inspect memory".to_string(),
            channel: "email".to_string(),
            reply_to: String::new(),
            tenant_id: String::new(),
            account_id: String::new(),
            attachment_refs: Vec::new(),
        });

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
    fn preserves_explicit_account_id_when_registry_is_empty() {
        let registry = AccountRegistry::default();
        let (request, resolved) = registry.resolve_create_request(CreateTaskRequest {
            customer_email: "dtang04@uchicago.edu".to_string(),
            subject: "Need help".to_string(),
            prompt: "Inspect memory".to_string(),
            channel: "email".to_string(),
            reply_to: "reply@example.com".to_string(),
            tenant_id: String::new(),
            account_id: "acct_manual".to_string(),
            attachment_refs: Vec::new(),
        });

        assert_eq!(request.account_id, "acct_manual");
        assert_eq!(
            resolved,
            ResolvedAccount {
                account_id: Some("acct_manual".to_string()),
                memory_path: None,
            }
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
