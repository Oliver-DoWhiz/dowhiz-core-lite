use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use crate::models::{CreateTaskRequest, InboundTaskRequest};

const IDENTIFIERS_DB_FILE: &str = "account_identifiers.sqlite3";
const MEMORY_PATHS_DB_FILE: &str = "account_memory_paths.sqlite3";

#[derive(Debug)]
pub struct AccountRegistry {
    identifiers_db_path: PathBuf,
    memory_paths_db_path: PathBuf,
    memory_root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountIdentifiers {
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub slack_user_ids: Vec<String>,
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
                write!(
                    f,
                    "email '{}' is already linked to account '{}'",
                    email, account_id
                )
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

impl From<rusqlite::Error> for AccountRegistryError {
    fn from(err: rusqlite::Error) -> Self {
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
        let configured_path = path.into();
        let registry_dir = configured_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let identifiers_db_path = registry_dir.join(IDENTIFIERS_DB_FILE);
        let memory_paths_db_path = registry_dir.join(MEMORY_PATHS_DB_FILE);

        initialize_store(&identifiers_db_path, &memory_paths_db_path)?;

        Ok(Self {
            identifiers_db_path,
            memory_paths_db_path,
            memory_root: default_memory_root(&configured_path),
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
        for _ in 0..32 {
            let suffix = Uuid::new_v4().simple().to_string();
            let candidate = format!("acct_{}", &suffix[..12]);
            if !account_exists(&self.identifiers_db_path, &candidate)? {
                return Ok(candidate);
            }
        }

        Err(anyhow!("failed to generate a unique account_id").into())
    }

    pub fn materialize_memory(
        &self,
        workspace_dir: &Path,
        resolved: &ResolvedAccount,
    ) -> Result<()> {
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
            reserve_or_update_requested_account(
                &self.identifiers_db_path,
                requested_account_id,
                customer_email,
                register_account_id,
            )?;
            let memory_path = self.ensure_memory_path(requested_account_id)?;
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

        let Some(account_id) =
            find_account_id_by_email(&self.identifiers_db_path, &normalized_email)?
        else {
            return Ok(ResolvedAccount {
                account_id: None,
                memory_path: None,
            });
        };

        let memory_path = self.ensure_memory_path(&account_id)?;
        Ok(ResolvedAccount {
            account_id: Some(account_id),
            memory_path: Some(memory_path),
        })
    }

    fn ensure_memory_path(
        &self,
        account_id: &str,
    ) -> std::result::Result<PathBuf, AccountRegistryError> {
        let default_memory_path = self
            .memory_root
            .join(sanitize_account_id_segment(account_id))
            .display()
            .to_string();
        let stored_memory_path = ensure_memory_path_record(
            &self.memory_paths_db_path,
            account_id,
            &default_memory_path,
        )?;
        let memory_path = PathBuf::from(stored_memory_path.trim());
        ensure_memory_source(&memory_path)?;
        Ok(memory_path)
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

fn initialize_store(identifiers_db_path: &Path, memory_paths_db_path: &Path) -> Result<()> {
    let conn = open_attached_connection(identifiers_db_path, memory_paths_db_path)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS account_identifiers (
            account_id TEXT PRIMARY KEY,
            emails_json TEXT NOT NULL DEFAULT '[]',
            phones_json TEXT NOT NULL DEFAULT '[]',
            slack_user_ids_json TEXT NOT NULL DEFAULT '[]',
            discord_user_ids_json TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS account_emails (
            email TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            FOREIGN KEY(account_id) REFERENCES account_identifiers(account_id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_account_emails_account_id
            ON account_emails(account_id);

        CREATE TABLE IF NOT EXISTS memory_registry.account_memory_paths (
            account_id TEXT PRIMARY KEY,
            memory_path TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

fn reserve_or_update_requested_account(
    identifiers_db_path: &Path,
    account_id: &str,
    customer_email: &str,
    register_account_id: bool,
) -> std::result::Result<(), AccountRegistryError> {
    let mut conn = open_identifiers_connection(identifiers_db_path)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let normalized_email = normalize_identifier(customer_email);
    if !normalized_email.is_empty() {
        if let Some(bound_account_id) = find_email_binding_tx(&tx, &normalized_email)? {
            if bound_account_id != account_id {
                return Err(AccountRegistryError::EmailAlreadyBound {
                    email: customer_email.trim().to_string(),
                    account_id: bound_account_id,
                });
            }
        }
    }

    let existing_identifiers = load_account_identifiers_tx(&tx, account_id)?;
    if register_account_id && existing_identifiers.is_some() {
        return Err(AccountRegistryError::AccountIdTaken(account_id.to_string()));
    }

    let mut identifiers = existing_identifiers.unwrap_or_default();
    let should_persist = append_email(&mut identifiers.emails, customer_email)
        || !account_exists_tx(&tx, account_id)?;
    if should_persist {
        upsert_account_identifiers_tx(&tx, account_id, &identifiers)?;
    }

    tx.commit()?;
    Ok(())
}

fn account_exists(
    identifiers_db_path: &Path,
    account_id: &str,
) -> std::result::Result<bool, AccountRegistryError> {
    let conn = open_identifiers_connection(identifiers_db_path)?;
    account_exists_conn(&conn, account_id)
}

fn account_exists_conn(
    conn: &Connection,
    account_id: &str,
) -> std::result::Result<bool, AccountRegistryError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM account_identifiers WHERE account_id = ?1 LIMIT 1",
            params![account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn account_exists_tx(
    tx: &Transaction<'_>,
    account_id: &str,
) -> std::result::Result<bool, AccountRegistryError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM account_identifiers WHERE account_id = ?1 LIMIT 1",
            params![account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn find_account_id_by_email(
    identifiers_db_path: &Path,
    normalized_email: &str,
) -> std::result::Result<Option<String>, AccountRegistryError> {
    let conn = open_identifiers_connection(identifiers_db_path)?;
    Ok(conn
        .query_row(
            "SELECT account_id FROM account_emails WHERE email = ?1",
            params![normalized_email],
            |row| row.get(0),
        )
        .optional()?)
}

fn find_email_binding_tx(
    tx: &Transaction<'_>,
    normalized_email: &str,
) -> std::result::Result<Option<String>, AccountRegistryError> {
    Ok(tx
        .query_row(
            "SELECT account_id FROM account_emails WHERE email = ?1",
            params![normalized_email],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
fn load_account_identifiers(
    identifiers_db_path: &Path,
    account_id: &str,
) -> std::result::Result<Option<AccountIdentifiers>, AccountRegistryError> {
    let conn = open_identifiers_connection(identifiers_db_path)?;
    load_account_identifiers_conn(&conn, account_id)
}

#[cfg(test)]
fn load_account_identifiers_conn(
    conn: &Connection,
    account_id: &str,
) -> std::result::Result<Option<AccountIdentifiers>, AccountRegistryError> {
    Ok(conn
        .query_row(
            "
            SELECT emails_json, phones_json, slack_user_ids_json, discord_user_ids_json
            FROM account_identifiers
            WHERE account_id = ?1
            ",
            params![account_id],
            |row| {
                Ok(AccountIdentifiers {
                    emails: decode_json_vec(row.get::<_, String>(0)?)?,
                    phones: decode_json_vec(row.get::<_, String>(1)?)?,
                    slack_user_ids: decode_json_vec(row.get::<_, String>(2)?)?,
                    discord_user_ids: decode_json_vec(row.get::<_, String>(3)?)?,
                })
            },
        )
        .optional()?)
}

fn load_account_identifiers_tx(
    tx: &Transaction<'_>,
    account_id: &str,
) -> std::result::Result<Option<AccountIdentifiers>, AccountRegistryError> {
    Ok(tx
        .query_row(
            "
            SELECT emails_json, phones_json, slack_user_ids_json, discord_user_ids_json
            FROM account_identifiers
            WHERE account_id = ?1
            ",
            params![account_id],
            |row| {
                Ok(AccountIdentifiers {
                    emails: decode_json_vec(row.get::<_, String>(0)?)?,
                    phones: decode_json_vec(row.get::<_, String>(1)?)?,
                    slack_user_ids: decode_json_vec(row.get::<_, String>(2)?)?,
                    discord_user_ids: decode_json_vec(row.get::<_, String>(3)?)?,
                })
            },
        )
        .optional()?)
}

fn upsert_account_identifiers_tx(
    tx: &Transaction<'_>,
    account_id: &str,
    identifiers: &AccountIdentifiers,
) -> std::result::Result<(), AccountRegistryError> {
    tx.execute(
        "
        INSERT INTO account_identifiers (
            account_id,
            emails_json,
            phones_json,
            slack_user_ids_json,
            discord_user_ids_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(account_id) DO UPDATE SET
            emails_json = excluded.emails_json,
            phones_json = excluded.phones_json,
            slack_user_ids_json = excluded.slack_user_ids_json,
            discord_user_ids_json = excluded.discord_user_ids_json
        ",
        params![
            account_id,
            encode_json_vec(&identifiers.emails)?,
            encode_json_vec(&identifiers.phones)?,
            encode_json_vec(&identifiers.slack_user_ids)?,
            encode_json_vec(&identifiers.discord_user_ids)?,
        ],
    )?;

    tx.execute(
        "DELETE FROM account_emails WHERE account_id = ?1",
        params![account_id],
    )?;

    let mut seen = HashSet::new();
    for email in &identifiers.emails {
        let normalized_email = normalize_identifier(email);
        if normalized_email.is_empty() || !seen.insert(normalized_email.clone()) {
            continue;
        }

        tx.execute(
            "INSERT OR REPLACE INTO account_emails (email, account_id) VALUES (?1, ?2)",
            params![normalized_email, account_id],
        )?;
    }

    Ok(())
}

fn ensure_memory_path_record(
    memory_paths_db_path: &Path,
    account_id: &str,
    default_memory_path: &str,
) -> std::result::Result<String, AccountRegistryError> {
    let mut conn = open_memory_connection(memory_paths_db_path)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(existing_memory_path) = tx
        .query_row(
            "SELECT memory_path FROM account_memory_paths WHERE account_id = ?1",
            params![account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        tx.commit()?;
        return Ok(existing_memory_path);
    }

    tx.execute(
        "INSERT INTO account_memory_paths (account_id, memory_path) VALUES (?1, ?2)",
        params![account_id, default_memory_path],
    )?;
    tx.commit()?;
    Ok(default_memory_path.to_string())
}

fn open_identifiers_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)
        .with_context(|| format!("failed to open account identifiers db {}", path.display()))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

fn open_memory_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    Connection::open(path)
        .with_context(|| format!("failed to open account memory paths db {}", path.display()))
}

fn open_attached_connection(
    identifiers_db_path: &Path,
    memory_paths_db_path: &Path,
) -> Result<Connection> {
    let conn = open_identifiers_connection(identifiers_db_path)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memory_registry",
        params![memory_paths_db_path.display().to_string()],
    )
    .with_context(|| {
        format!(
            "failed to attach account memory paths db {}",
            memory_paths_db_path.display()
        )
    })?;
    Ok(conn)
}

fn encode_json_vec(values: &[String]) -> Result<String> {
    serde_json::to_string(values).context("failed to encode identifier array")
}

fn decode_json_vec(value: String) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(&value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
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
        Err(AccountRegistryError::InvalidAccountId(
            account_id.to_string(),
        ))
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

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::models::CreateTaskRequest;

    #[test]
    fn ignores_legacy_json_registry_and_starts_with_empty_sqlite() {
        let root = temp_dir("account-registry-migrate");
        let registry_path = root.join("account_registry.json");
        fs::write(
            &registry_path,
            r#"{
  "identifiers_by_account_id": {
    "acct_123": {
      "emails": ["dtang04@uchicago.edu"],
      "phones": ["+16309153426"],
      "slack_user_ids": ["U0AG9M23K1R"],
      "discord_user_ids": []
    }
  },
  "memory_path_by_account_id": {
    "acct_123": "ignored"
  }
}"#,
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

        assert!(request.account_id.is_empty());
        assert!(request.identity_uri.is_empty());
        assert_eq!(resolved.account_id, None);
        assert_eq!(resolved.memory_path, None);
    }

    #[test]
    fn resolves_account_by_email_and_injects_memory_path() {
        let root = temp_dir("account-registry");
        let memory_dir = root.join("memory-source");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("memo.md"), "# hello").unwrap();

        let registry_path = root.join("account_registry.json");
        let registry = AccountRegistry::load(&registry_path).unwrap();
        registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Need help".to_string(),
                prompt: "Register memory".to_string(),
                channel: "email".to_string(),
                reply_to: String::new(),
                tenant_id: String::new(),
                account_id: "acct_123".to_string(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();
        set_memory_path(&root, "acct_123", &memory_dir);

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
        assert_eq!(resolved.memory_path, Some(memory_dir.clone()));

        let workspace_dir = root.join("workspace");
        registry
            .materialize_memory(&workspace_dir, &resolved)
            .unwrap();
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
        assert_eq!(resolved.account_id, Some("acct_manual".to_string()));
        let memory_path = resolved.memory_path.unwrap();
        assert_eq!(
            memory_path,
            root.join("account_memories").join("acct_manual")
        );
        assert_eq!(
            fs::read_to_string(memory_path.join("memo.md")).unwrap(),
            "# Memo\n"
        );

        let identifiers = load_account_identifiers(&root.join(IDENTIFIERS_DB_FILE), "acct_manual")
            .unwrap()
            .unwrap();
        assert_eq!(identifiers.emails, vec!["dtang04@uchicago.edu".to_string()]);
        assert_eq!(
            ensure_memory_path_record(
                &root.join(MEMORY_PATHS_DB_FILE),
                "acct_manual",
                "ignored-default"
            )
            .unwrap(),
            root.join("account_memories")
                .join("acct_manual")
                .display()
                .to_string()
        );
    }

    #[test]
    fn rejects_registering_an_already_taken_account_id() {
        let root = temp_dir("account-registry-taken");
        let registry_path = root.join("account_registry.json");
        let registry = AccountRegistry::load(&registry_path).unwrap();
        registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "existing@example.com".to_string(),
                subject: "Seed account".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: "reply@example.com".to_string(),
                tenant_id: String::new(),
                account_id: "acct_manual".to_string(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();

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

        assert!(
            matches!(err, AccountRegistryError::AccountIdTaken(ref value) if value == "acct_manual")
        );
    }

    #[test]
    fn appends_new_email_to_existing_account() {
        let root = temp_dir("account-registry-email");
        let registry_path = root.join("account_registry.json");
        let registry = AccountRegistry::load(&registry_path).unwrap();
        registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dylan@dowhiz.com".to_string(),
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

        let identifiers = load_account_identifiers(&root.join(IDENTIFIERS_DB_FILE), "acct_manual")
            .unwrap()
            .unwrap();
        assert_eq!(
            identifiers.emails,
            vec![
                "dylan@dowhiz.com".to_string(),
                "dtang04@uchicago.edu".to_string()
            ]
        );
    }

    #[test]
    fn initializes_missing_memo_for_existing_sqlite_memory_path_without_creating_legacy_file() {
        let root = temp_dir("account-registry-init-sqlite-memory");
        let memory_dir = root.join("memory-source");
        fs::create_dir_all(&memory_dir).unwrap();

        let registry_path = root.join("account_registry.json");
        let registry = AccountRegistry::load(&registry_path).unwrap();
        registry
            .resolve_create_request(CreateTaskRequest {
                customer_email: "dtang04@uchicago.edu".to_string(),
                subject: "Seed account".to_string(),
                prompt: "Inspect memory".to_string(),
                channel: "email".to_string(),
                reply_to: String::new(),
                tenant_id: String::new(),
                account_id: "acct_123".to_string(),
                register_account_id: false,
                attachment_refs: Vec::new(),
            })
            .unwrap();
        set_memory_path(&root, "acct_123", &memory_dir);

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
        assert!(!registry_path.exists());
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

        assert_eq!(
            fs::read_to_string(source_dir.join("memo.md")).unwrap(),
            "# Updated"
        );
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

    fn set_memory_path(root: &Path, account_id: &str, memory_path: &Path) {
        let conn = open_memory_connection(&root.join(MEMORY_PATHS_DB_FILE)).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO account_memory_paths (account_id, memory_path) VALUES (?1, ?2)",
            params![account_id, memory_path.display().to_string()],
        )
        .unwrap();
    }
}
