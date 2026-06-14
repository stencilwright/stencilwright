//! Daemon-owned secret resolution for provider references.
//!
//! Short-lived clients send references and non-secret mapping config;
//! the daemon resolves values only when it needs to fill the browser,
//! interpolate a URL, or build the value-name map for masking.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use stencil_core::ValuesConfig;
use stencil_mask::ValueNameMap;
use stencil_secrets::{
    DiscoveredSecretItem, OnePasswordProvider, OpConfig, SecretDiscoveryQuery, SecretProvider,
    is_credential_reference, is_secret_reference, is_totp_reference,
};

#[derive(Debug, Default)]
pub(crate) struct SecretResolver {
    cache: HashMap<String, String>,
    op_config: OpConfig,
}

impl SecretResolver {
    pub(crate) fn new(op_config: OpConfig) -> Self {
        Self {
            cache: HashMap::new(),
            op_config,
        }
    }

    pub(crate) async fn resolve_spec(
        &mut self,
        spec: &str,
        values: &ValuesConfig,
    ) -> Result<String> {
        if is_secret_reference(spec) {
            self.resolve_uri(spec).await
        } else if let Some(name) = spec.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let uri = values
                .entries
                .get(name)
                .ok_or_else(|| anyhow!("unknown values.toml reference: {{{name}}}"))?;
            self.resolve_uri(uri).await
        } else {
            Err(anyhow!(
                "secret reference must be a secret:// provider reference or {{name}} reference, got: {spec}"
            ))
        }
    }

    pub(crate) async fn interpolate(
        &mut self,
        template: &str,
        values: &ValuesConfig,
    ) -> Result<String> {
        let mut out = template.to_string();
        for (name, uri) in &values.entries {
            let needle = format!("{{{name}}}");
            if out.contains(&needle) {
                let value = self.resolve_uri(uri).await?;
                out = out.replace(&needle, &value);
            }
        }
        Ok(out)
    }

    pub(crate) async fn template_url(&mut self, url: &str, values: &ValuesConfig) -> String {
        let mut replacements = Vec::new();
        for (name, uri) in &values.entries {
            if is_totp_reference(uri) {
                continue;
            }
            if let Ok(value) = self.resolve_uri(uri).await {
                if !value.is_empty() && url.contains(&value) {
                    replacements.push((value, format!("{{{name}}}")));
                }
            }
        }
        replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        let mut out = url.to_string();
        for (value, name) in replacements {
            out = out.replace(&value, &name);
        }
        out
    }

    pub(crate) async fn value_name_map(&mut self, values: &ValuesConfig) -> ValueNameMap {
        let mut out = ValueNameMap::new();
        for (name, uri) in &values.entries {
            if is_totp_reference(uri) || is_credential_reference(uri) {
                continue;
            }
            if let Ok(value) = self.resolve_uri(uri).await {
                out.insert(value, name.clone());
            }
        }
        out
    }

    pub(crate) async fn discover(
        &self,
        query: &SecretDiscoveryQuery,
    ) -> Result<Vec<DiscoveredSecretItem>> {
        let provider = OnePasswordProvider::new(self.op_config.clone());
        provider.discover_items(query).await
    }

    async fn resolve_uri(&mut self, uri: &str) -> Result<String> {
        if is_totp_reference(uri) {
            return stencil_secrets::read_secret_with_config(uri, &self.op_config).await;
        }
        if let Some(cached) = self.cache.get(uri) {
            return Ok(cached.clone());
        }
        let value = stencil_secrets::read_secret_with_config(uri, &self.op_config).await?;
        self.cache.insert(uri.to_string(), value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_uri_detection_is_field_based() {
        assert!(is_credential_reference(
            "secret://1password/vault/item/username"
        ));
        assert!(is_credential_reference(
            "secret://1password/vault/item/password"
        ));
        assert!(is_credential_reference(
            "secret://1password/vault/item/otp?"
        ));
        assert!(!is_credential_reference(
            "secret://1password/vault/item/account_number"
        ));
    }
}
