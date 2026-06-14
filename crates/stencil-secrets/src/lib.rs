//! Secret resolution and discovery via local secret-provider CLIs.
//!
//! See `specs/01-stencil.md` §8. Pure shell-out — we never store
//! resolved values on disk. The `stencilwright` daemon owns session
//! resolution and caches non-TOTP values in memory; short-lived CLI
//! clients send references, not resolved secret strings.
//!
//! The first provider is 1Password. Multi-account support: every `op`
//! invocation can pass `--account` from non-secret site config.
//! `OP_ACCOUNT` remains a fallback for ad-hoc commands.
//!
//! Provider CLIs are contacted only from daemon-owned first-use paths:
//! filling a secret, interpolating a secret, masking a configured
//! non-credential value, or user-only provider discovery.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stencil_core::SiteConfig;
use tokio::process::Command;

type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// 1Password CLI selection options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpConfig {
    account: Option<String>,
}

impl OpConfig {
    pub fn new(account: Option<String>) -> Self {
        Self {
            account: clean_account(account),
        }
    }

    pub fn from_site_config(config: &SiteConfig) -> Self {
        Self::new(config.onepassword_account.clone())
    }

    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretProviderId {
    OnePassword,
}

/// A secret provider can discover user-visible candidate items and
/// resolve an opaque stored reference. Discovery results are meant for
/// user-only UI surfaces; do not print them to stdout.
pub trait SecretProvider {
    fn id(&self) -> SecretProviderId;

    fn discover_items<'a>(
        &'a self,
        query: &'a SecretDiscoveryQuery,
    ) -> ProviderFuture<'a, Vec<DiscoveredSecretItem>>;

    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> ProviderFuture<'a, String>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretDiscoveryQuery {
    pub search: Option<String>,
    pub vault: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretReference {
    raw: String,
    kind: SecretReferenceKind,
}

#[derive(Clone, PartialEq, Eq)]
enum SecretReferenceKind {
    OnePasswordItem {
        vault_id: String,
        item_id: String,
        field: String,
        otp: bool,
    },
}

impl SecretReference {
    pub fn parse(raw: &str) -> Result<Self> {
        let Some(rest) = raw.strip_prefix("secret://1password/") else {
            bail!("unsupported secret reference provider");
        };
        let rest_without_otp = rest.strip_suffix('?').unwrap_or(rest);
        let otp = rest.ends_with('?');
        let parts = rest_without_otp.split('/').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
            bail!("secret://1password reference must include vault, item, and field");
        }
        let vault_id = decode_component(parts[0]).context("decoding 1Password vault id")?;
        let item_id = decode_component(parts[1]).context("decoding 1Password item id")?;
        let field = decode_component(parts[2]).context("decoding 1Password field")?;
        Ok(Self {
            raw: raw.to_string(),
            kind: SecretReferenceKind::OnePasswordItem {
                vault_id,
                item_id,
                field,
                otp,
            },
        })
    }

    pub fn onepassword_item_field(vault_id: &str, item_id: &str, field: &str) -> Self {
        let raw = format!(
            "secret://1password/{}/{}/{}",
            encode_component(vault_id),
            encode_component(item_id),
            encode_component(field)
        );
        Self {
            raw,
            kind: SecretReferenceKind::OnePasswordItem {
                vault_id: vault_id.to_string(),
                item_id: item_id.to_string(),
                field: field.to_string(),
                otp: false,
            },
        }
    }

    pub fn onepassword_item_otp(vault_id: &str, item_id: &str) -> Self {
        let raw = format!(
            "secret://1password/{}/{}/otp?",
            encode_component(vault_id),
            encode_component(item_id)
        );
        Self {
            raw,
            kind: SecretReferenceKind::OnePasswordItem {
                vault_id: vault_id.to_string(),
                item_id: item_id.to_string(),
                field: "otp".to_string(),
                otp: true,
            },
        }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn provider(&self) -> SecretProviderId {
        SecretProviderId::OnePassword
    }

    pub fn is_otp(&self) -> bool {
        match &self.kind {
            SecretReferenceKind::OnePasswordItem { otp, .. } => *otp,
        }
    }

    pub fn is_credential(&self) -> bool {
        match &self.kind {
            SecretReferenceKind::OnePasswordItem { field, otp, .. } => {
                *otp || matches!(
                    field.as_str(),
                    "username" | "password" | "one-time password"
                )
            }
        }
    }
}

impl fmt::Debug for SecretReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretReference")
            .field("provider", &self.provider())
            .field("otp", &self.is_otp())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredSecretItem {
    provider: SecretProviderId,
    item_id: String,
    title: String,
    vault_id: String,
    vault_name: Option<String>,
    category: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    urls: Vec<DiscoveredSecretUrl>,
    references: Vec<DiscoveredSecretReference>,
}

impl DiscoveredSecretItem {
    pub fn provider(&self) -> SecretProviderId {
        self.provider
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }

    pub fn vault_name(&self) -> Option<&str> {
        self.vault_name.as_deref()
    }

    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    pub fn created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }

    pub fn updated_at(&self) -> Option<&str> {
        self.updated_at.as_deref()
    }

    pub fn urls(&self) -> &[DiscoveredSecretUrl] {
        &self.urls
    }

    pub fn references(&self) -> &[DiscoveredSecretReference] {
        &self.references
    }

    pub fn field_reference(&self, field: &str) -> SecretReference {
        SecretReference::onepassword_item_field(&self.vault_id, &self.item_id, field)
    }

    pub fn otp_reference(&self) -> SecretReference {
        SecretReference::onepassword_item_otp(&self.vault_id, &self.item_id)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredSecretUrl {
    label: Option<String>,
    primary: bool,
    href: String,
}

impl DiscoveredSecretUrl {
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn primary(&self) -> bool {
        self.primary
    }

    pub fn href(&self) -> &str {
        &self.href
    }
}

impl fmt::Debug for DiscoveredSecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveredSecretUrl")
            .field("label", &self.label)
            .field("primary", &self.primary)
            .field("href", &"<redacted>")
            .finish()
    }
}

impl fmt::Debug for DiscoveredSecretItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveredSecretItem")
            .field("provider", &self.provider)
            .field("category", &self.category)
            .field("updated_at", &self.updated_at)
            .field("title", &"<redacted>")
            .field("vault", &"<redacted>")
            .field("urls", &self.urls.len())
            .field("references", &self.references.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredSecretReference {
    field: String,
    reference: String,
}

impl DiscoveredSecretReference {
    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }
}

impl fmt::Debug for DiscoveredSecretReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveredSecretReference")
            .field("field", &self.field)
            .field("reference", &"<opaque>")
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct OnePasswordProvider {
    config: OpConfig,
}

impl OnePasswordProvider {
    pub fn new(config: OpConfig) -> Self {
        Self { config }
    }
}

impl SecretProvider for OnePasswordProvider {
    fn id(&self) -> SecretProviderId {
        SecretProviderId::OnePassword
    }

    fn discover_items<'a>(
        &'a self,
        query: &'a SecretDiscoveryQuery,
    ) -> ProviderFuture<'a, Vec<DiscoveredSecretItem>> {
        Box::pin(async move { discover_onepassword_items(&self.config, query).await })
    }

    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> ProviderFuture<'a, String> {
        Box::pin(async move { resolve_onepassword_reference(reference, &self.config).await })
    }
}

pub async fn read_secret_with_config(raw: &str, config: &OpConfig) -> Result<String> {
    let reference = SecretReference::parse(raw)?;
    let provider = OnePasswordProvider::new(config.clone());
    provider.resolve(&reference).await
}

pub fn validate_reference(raw: &str) -> Result<()> {
    SecretReference::parse(raw).map(|_| ())
}

pub fn is_secret_reference(raw: &str) -> bool {
    SecretReference::parse(raw).is_ok()
}

pub fn is_totp_reference(raw: &str) -> bool {
    SecretReference::parse(raw).is_ok_and(|reference| reference.is_otp())
}

pub fn is_credential_reference(raw: &str) -> bool {
    SecretReference::parse(raw).is_ok_and(|reference| reference.is_credential())
}

async fn resolve_onepassword_reference(
    reference: &SecretReference,
    config: &OpConfig,
) -> Result<String> {
    match &reference.kind {
        SecretReferenceKind::OnePasswordItem {
            vault_id,
            item_id,
            field,
            otp,
        } => {
            if *otp {
                read_otp(vault_id, item_id, config).await
            } else {
                read_item_field(vault_id, item_id, field, config).await
            }
        }
    }
}

async fn read_otp(vault: &str, item: &str, config: &OpConfig) -> Result<String> {
    let out = op_cmd(config)
        .arg("item")
        .arg("get")
        .arg(item)
        .arg("--vault")
        .arg(vault)
        .arg("--otp")
        .output()
        .await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "op item get --otp failed for {vault}/{item}: {stderr}"
        ));
    }
    let code = String::from_utf8(out.stdout)?
        .trim_end_matches('\n')
        .to_string();
    Ok(code)
}

async fn read_item_field(
    vault_id: &str,
    item_id: &str,
    field: &str,
    config: &OpConfig,
) -> Result<String> {
    let mut cmd = op_cmd(config);
    for arg in item_field_get_args(item_id, vault_id, field) {
        cmd.arg(arg);
    }
    let out = cmd.output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("op item get field failed: {stderr}"));
    }
    Ok(String::from_utf8(out.stdout)?
        .trim_end_matches('\n')
        .to_string())
}

fn item_field_get_args(item_id: &str, vault_id: &str, field: &str) -> Vec<String> {
    vec![
        "item".to_string(),
        "get".to_string(),
        item_id.to_string(),
        "--vault".to_string(),
        vault_id.to_string(),
        "--fields".to_string(),
        format!("label={field}"),
        "--reveal".to_string(),
    ]
}

async fn discover_onepassword_items(
    config: &OpConfig,
    query: &SecretDiscoveryQuery,
) -> Result<Vec<DiscoveredSecretItem>> {
    let mut cmd = op_cmd(config);
    cmd.arg("item")
        .arg("list")
        .arg("--format")
        .arg("json")
        .arg("--long");
    if let Some(vault) = query.vault.as_deref().filter(|vault| !vault.is_empty()) {
        cmd.arg("--vault").arg(vault);
    }
    if !query.categories.is_empty() {
        cmd.arg("--categories").arg(query.categories.join(","));
    }

    let out = cmd.output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("op item list failed: {stderr}"));
    }

    let values =
        serde_json::from_slice::<Vec<Value>>(&out.stdout).context("parsing op item list JSON")?;
    let search = query
        .search
        .as_deref()
        .filter(|search| !search.trim().is_empty())
        .map(|search| search.trim().to_ascii_lowercase());

    let mut items = Vec::new();
    for value in values {
        let Some(item) = discovered_item_from_value(&value) else {
            continue;
        };
        if let Some(search) = &search {
            let haystack = discovery_haystack(&item);
            if !haystack.contains(search) {
                continue;
            }
        }
        items.push(item);
    }
    items.sort_by(compare_discovered_items);
    Ok(items)
}

fn discovered_item_from_value(value: &Value) -> Option<DiscoveredSecretItem> {
    let item_id = value.get("id")?.as_str()?.to_string();
    let title = value.get("title")?.as_str()?.to_string();
    let vault = value.get("vault")?;
    let vault_id = value_string(vault.get("id")?)?;
    let vault_name = vault.get("name").and_then(value_string);
    let category = value.get("category").and_then(value_string);
    let created_at = value.get("created_at").and_then(value_string);
    let updated_at = value.get("updated_at").and_then(value_string);
    let urls = value
        .get("urls")
        .and_then(Value::as_array)
        .map(|urls| urls.iter().filter_map(discovered_url_from_value).collect())
        .unwrap_or_default();
    let mut item = DiscoveredSecretItem {
        provider: SecretProviderId::OnePassword,
        item_id,
        title,
        vault_id,
        vault_name,
        category,
        created_at,
        updated_at,
        urls,
        references: vec![],
    };
    item.references = discovered_references_for(&item);
    Some(item)
}

fn discovered_references_for(item: &DiscoveredSecretItem) -> Vec<DiscoveredSecretReference> {
    ["username", "password", "otp"]
        .into_iter()
        .map(|field| {
            let reference = if field == "otp" {
                item.otp_reference()
            } else {
                item.field_reference(field)
            };
            DiscoveredSecretReference {
                field: field.to_string(),
                reference: reference.as_str().to_string(),
            }
        })
        .collect()
}

fn discovered_url_from_value(value: &Value) -> Option<DiscoveredSecretUrl> {
    let href = value.get("href")?.as_str()?.to_string();
    if href.trim().is_empty() {
        return None;
    }
    Some(DiscoveredSecretUrl {
        label: value.get("label").and_then(value_string),
        primary: value
            .get("primary")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        href,
    })
}

fn discovery_haystack(item: &DiscoveredSecretItem) -> String {
    let urls = item
        .urls
        .iter()
        .map(|url| {
            format!(
                "{} {} {}",
                url.label.as_deref().unwrap_or_default(),
                url.primary,
                url.href
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} {} {} {}",
        item.title,
        item.vault_name.as_deref().unwrap_or_default(),
        item.category.as_deref().unwrap_or_default(),
        urls,
    )
    .to_ascii_lowercase()
}

fn compare_discovered_items(
    a: &DiscoveredSecretItem,
    b: &DiscoveredSecretItem,
) -> std::cmp::Ordering {
    b.updated_at
        .cmp(&a.updated_at)
        .then_with(|| a.title.cmp(&b.title))
        .then_with(|| a.item_id.cmp(&b.item_id))
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| map.get("id").and_then(Value::as_str))
            .map(ToOwned::to_owned),
        _ => None,
    }
}

/// `Command::new("op")` plus `--account` from config or OP_ACCOUNT.
fn op_cmd(config: &OpConfig) -> Command {
    let mut cmd = Command::new("op");
    if let Some(account) = selected_account(config) {
        cmd.arg("--account").arg(account);
    }
    cmd
}

fn selected_account(config: &OpConfig) -> Option<String> {
    config.account().map(ToOwned::to_owned).or_else(|| {
        std::env::var("OP_ACCOUNT")
            .ok()
            .and_then(|account| clean_account(Some(account)))
    })
}

fn clean_account(account: Option<String>) -> Option<String> {
    account.and_then(|account| {
        let trimmed = account.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn encode_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode_component(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                bail!("incomplete percent escape");
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
            let byte = u8::from_str_radix(hex, 16).context("invalid percent escape")?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_config_uses_trimmed_site_account() {
        let config = SiteConfig {
            onepassword_account: Some("  my.1password.com  ".to_string()),
        };
        let op_config = OpConfig::from_site_config(&config);
        assert_eq!(op_config.account(), Some("my.1password.com"));
    }

    #[test]
    fn op_config_treats_blank_account_as_absent() {
        let config = OpConfig::new(Some("   ".to_string()));
        assert_eq!(config.account(), None);
    }

    #[test]
    fn parses_opaque_1password_references() {
        let reference =
            SecretReference::onepassword_item_field("vault id", "item/id", "one-time password");
        assert_eq!(
            reference.as_str(),
            "secret://1password/vault%20id/item%2Fid/one-time%20password"
        );

        let reparsed = SecretReference::parse(reference.as_str()).unwrap();
        assert_eq!(reparsed, reference);
        assert!(reparsed.is_credential());
    }

    #[test]
    fn otp_reference_uses_trailing_sentinel() {
        let reference = SecretReference::onepassword_item_otp("vault", "item");
        assert_eq!(reference.as_str(), "secret://1password/vault/item/otp?");
        assert!(is_totp_reference(reference.as_str()));
        assert!(is_credential_reference(reference.as_str()));
    }

    #[test]
    fn item_field_reads_reveal_concealed_values() {
        let args = item_field_get_args("item-id", "vault-id", "password");
        assert_eq!(
            args,
            vec![
                "item",
                "get",
                "item-id",
                "--vault",
                "vault-id",
                "--fields",
                "label=password",
                "--reveal"
            ]
        );
    }

    #[test]
    fn rejects_unsupported_provider_references() {
        assert!(SecretReference::parse("secret://unsupported/item/field").is_err());
        assert!(!is_secret_reference("secret://unsupported/item/field"));
    }

    #[test]
    fn debug_redacts_discovered_item_names() {
        let item = DiscoveredSecretItem {
            provider: SecretProviderId::OnePassword,
            item_id: "item".to_string(),
            title: "Sensitive Login".to_string(),
            vault_id: "vault".to_string(),
            vault_name: Some("Private".to_string()),
            category: Some("LOGIN".to_string()),
            created_at: None,
            updated_at: None,
            urls: vec![],
            references: vec![],
        };

        let debug = format!("{item:?}");
        assert!(!debug.contains("Sensitive Login"));
        assert!(!debug.contains("Private"));
    }

    #[test]
    fn parses_item_list_entries_without_values() {
        let value = serde_json::json!({
            "id": "item-id",
            "title": "Example",
            "category": "LOGIN",
            "vault": { "id": "vault-id", "name": "Personal" }
        });

        let item = discovered_item_from_value(&value).unwrap();
        assert_eq!(item.item_id(), "item-id");
        assert_eq!(item.title(), "Example");
        assert_eq!(item.vault_id(), "vault-id");
        assert_eq!(item.vault_name(), Some("Personal"));
        assert_eq!(item.category(), Some("LOGIN"));
        assert_eq!(
            item.field_reference("username").as_str(),
            "secret://1password/vault-id/item-id/username"
        );
    }

    #[test]
    fn parses_urls_and_generated_references_from_item_list() {
        let value = serde_json::json!({
            "id": "item-id",
            "title": "Example",
            "category": "LOGIN",
            "vault": { "id": "vault-id", "name": "Private" },
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "urls": [
                { "label": "website", "primary": true, "href": "https://example.com/login" }
            ]
        });

        let item = discovered_item_from_value(&value).unwrap();
        assert_eq!(item.updated_at(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(item.urls().len(), 1);
        assert_eq!(item.urls()[0].href(), "https://example.com/login");
        let references = item
            .references()
            .iter()
            .map(|reference| (reference.field(), reference.reference()))
            .collect::<Vec<_>>();
        assert!(references.contains(&("username", "secret://1password/vault-id/item-id/username")));
        assert!(references.contains(&("password", "secret://1password/vault-id/item-id/password")));
        assert!(references.contains(&("otp", "secret://1password/vault-id/item-id/otp?")));
    }

    #[test]
    fn discovered_items_sort_updated_desc_then_title() {
        let mut items = [
            ("old", "Old", Some("2024-01-01T00:00:00Z")),
            ("new-b", "Beta", Some("2026-01-01T00:00:00Z")),
            ("new-a", "Alpha", Some("2026-01-01T00:00:00Z")),
            ("missing", "Missing", None),
        ]
        .into_iter()
        .map(|(item_id, title, updated_at)| DiscoveredSecretItem {
            provider: SecretProviderId::OnePassword,
            item_id: item_id.to_string(),
            title: title.to_string(),
            vault_id: "vault".to_string(),
            vault_name: None,
            category: Some("LOGIN".to_string()),
            created_at: None,
            updated_at: updated_at.map(ToOwned::to_owned),
            urls: vec![],
            references: vec![],
        })
        .collect::<Vec<_>>();

        items.sort_by(compare_discovered_items);

        assert_eq!(
            items.iter().map(|item| item.item_id()).collect::<Vec<_>>(),
            vec!["new-a", "new-b", "old", "missing"]
        );
    }
}
