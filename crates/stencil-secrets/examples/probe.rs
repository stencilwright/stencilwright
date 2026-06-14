//! Manual probe: `OP_ACCOUNT=... cargo run -p stencil-secrets --example probe -- 'secret://1password/vault-id/item-id/field'`
//! Prints length + whether output is all digits — never echoes the value.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let reference = std::env::args()
        .nth(1)
        .expect("usage: probe <secret://provider/... reference>");
    let v = stencil_secrets::read_secret_with_config(&reference, &Default::default()).await?;
    let all_digits = !v.is_empty() && v.chars().all(|c| c.is_ascii_digit());
    println!("ok: len={} all_digits={}", v.len(), all_digits);
    Ok(())
}
