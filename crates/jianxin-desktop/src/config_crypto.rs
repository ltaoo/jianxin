//! Mobile v2 configuration format: Base64(nonce || ciphertext || GCM tag).
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use std::{fs::File, io::Read, num::NonZeroU32, path::Path};
use zeroize::Zeroizing;

const PLAINTEXT_LIMIT: usize = 1_400_000;
const KEY_CONTEXT: &[u8] = b"jianxin-config:password-key:v2:aes-256-gcm";
const AAD: &[u8] = b"jianxin-config:v2:pbkdf2-sha256:600000:aes-256-gcm";

pub fn validate_password(password: &str) -> Result<()> {
    if password.is_empty() || password.len() > 1024 {
        bail!("输入加密密钥，长度为 1 至 1024 字节");
    }
    Ok(())
}

pub fn encrypt_current_config(path: &Path, password: &str) -> Result<String> {
    validate_password(password)?;
    let file = File::open(path).context("无法读取当前配置，检查文件是否存在及读取权限后重试")?;
    let mut plaintext = Zeroizing::new(Vec::new());
    file.take((PLAINTEXT_LIMIT + 1) as u64)
        .read_to_end(&mut plaintext)
        .context("读取配置失败，检查文件后重试")?;
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("无法生成加密随机数，重试生成密文"))?;
    encrypt_with_nonce(&plaintext, password, nonce)
}

fn encryption_key(password: &str) -> Result<LessSafeKey> {
    let mut key = Zeroizing::new([0; 32]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(600_000).unwrap(),
        KEY_CONTEXT,
        password.as_bytes(),
        key.as_mut(),
    );
    UnboundKey::new(&AES_256_GCM, key.as_ref())
        .map(LessSafeKey::new)
        .map_err(|_| anyhow::anyhow!("无法创建加密密钥，重试生成密文"))
}

fn encrypt_with_nonce(plaintext: &[u8], password: &str, nonce: [u8; 12]) -> Result<String> {
    validate_password(password)?;
    if plaintext.is_empty() || plaintext.len() > PLAINTEXT_LIMIT {
        bail!("原始配置不能为空，最多 1.4 MB");
    }
    let config: serde_json::Value = serde_json::from_slice(plaintext)
        .map_err(|_| anyhow::anyhow!("当前配置不是有效 JSON，修正 config.json 后重试"))?;
    let accounts = config["accounts"]
        .as_array()
        .filter(|accounts| (1..=100).contains(&accounts.len()));
    if accounts.is_none() {
        bail!("配置需要包含 1 至 100 个账号，调整 config.json 后重试");
    }
    let key = encryption_key(password)?;
    let mut encrypted = Zeroizing::new(plaintext.to_vec());
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(AAD),
        &mut *encrypted,
    )
    .map_err(|_| anyhow::anyhow!("加密失败，重试生成密文"))?;
    let mut packed = nonce.to_vec();
    packed.extend_from_slice(&encrypted);
    Ok(STANDARD.encode(packed) + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../mobile/tests/fixtures/config.json")).unwrap()
    }

    fn decrypt(encoded: &str, password: &str) -> Result<Vec<u8>> {
        let packed = STANDARD.decode(encoded.trim())?;
        let nonce: [u8; 12] = packed[..12].try_into()?;
        let mut encrypted = packed[12..].to_vec();
        let plain = encryption_key(password)?
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(AAD),
                &mut encrypted,
            )
            .map_err(|_| anyhow::anyhow!("decryption failed"))?;
        Ok(plain.to_vec())
    }

    #[test]
    fn matches_mobile_node_swift_and_kotlin_unicode_vector() {
        let fixture = fixture();
        let plaintext = fixture["plaintext"].as_str().unwrap();
        let password = fixture["password"].as_str().unwrap();
        let nonce = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
        ];
        let encrypted = encrypt_with_nonce(plaintext.as_bytes(), password, nonce).unwrap();
        assert_eq!(
            encrypted,
            fixture["aes-256-gcm"]
                .as_str()
                .unwrap()
                .replace("\\n", "\n")
        );
        assert_eq!(decrypt(&encrypted, password).unwrap(), plaintext.as_bytes());
        assert!(decrypt(&encrypted, "wrong").is_err());
        let mut changed = STANDARD.decode(encrypted.trim()).unwrap();
        changed[12] ^= 1;
        assert!(decrypt(&STANDARD.encode(changed), password).is_err());
    }

    #[test]
    fn rejects_invalid_passwords_and_configs_before_encryption() {
        let valid = br#"{"accounts":[{"email":"demo@example.com"}]}"#;
        assert!(encrypt_with_nonce(valid, "", [0; 12]).is_err());
        assert!(validate_password(&"密".repeat(342)).is_err());
        for plaintext in [
            b"".as_slice(),
            b"not-json",
            br#"{"accounts":[]}"#,
            br#"{"accounts":{}}"#,
        ] {
            assert!(encrypt_with_nonce(plaintext, "password", [0; 12]).is_err());
        }
        assert!(encrypt_with_nonce(&vec![b' '; PLAINTEXT_LIMIT + 1], "password", [0; 12]).is_err());
        let config = serde_json::json!({"accounts": vec![serde_json::json!({}); 101]}).to_string();
        assert!(encrypt_with_nonce(config.as_bytes(), "password", [0; 12]).is_err());
    }

    #[test]
    fn reads_selected_file_again_and_uses_a_fresh_nonce() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "jianxin-crypto-test-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let first = br#"{ "accounts": [{"email":"first@example.com"}] }"#;
        let second = br#"{"accounts":[{"email":"second@example.com"}]}"#;
        std::fs::write(&path, first).unwrap();
        let a = encrypt_current_config(&path, "密钥🔐").unwrap();
        let b = encrypt_current_config(&path, "密钥🔐").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&a, "密钥🔐").unwrap(), first);
        assert_eq!(std::fs::read(&path).unwrap(), first);
        std::fs::write(&path, second).unwrap();
        let c = encrypt_current_config(&path, "密钥🔐").unwrap();
        assert_eq!(decrypt(&c, "密钥🔐").unwrap(), second);
        std::fs::remove_file(&path).unwrap();
        assert!(encrypt_current_config(&path, "密钥🔐").is_err());
    }
}
