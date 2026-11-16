//! 密码同步加密：用「同步口令」派生 AES-256-GCM 密钥，
//! 加密后的密码库随私有仓库同步，口令本身不入库。
//!
//! 密文格式：base64(salt[16] || nonce[12] || ciphertext)

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const ITERATIONS: u32 = 100_000;

fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut digest = {
        let mut h = Sha256::new();
        h.update(passphrase.as_bytes());
        h.update(salt);
        h.finalize()
    };
    for _ in 0..ITERATIONS {
        let mut h = Sha256::new();
        h.update(digest);
        h.update(passphrase.as_bytes());
        digest = h.finalize();
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// 加密明文（返回 base64 字符串）
#[allow(dead_code)]
pub fn encrypt(passphrase: &str, plaintext: &str) -> Result<String, String> {
    if passphrase.is_empty() {
        return Err("同步口令为空".into());
    }
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化加密器失败: {}", e))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;

    let mut blob = Vec::with_capacity(SALT_LEN + NONCE_LEN + ct.len());
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct);
    Ok(B64.encode(blob))
}

/// 解密（输入 base64 字符串）
#[allow(dead_code)]
pub fn decrypt(passphrase: &str, encoded: &str) -> Result<String, String> {
    if passphrase.is_empty() {
        return Err("同步口令为空".into());
    }
    let blob = B64
        .decode(encoded.trim())
        .map_err(|e| format!("密文格式错误: {}", e))?;
    if blob.len() < SALT_LEN + NONCE_LEN + 16 {
        return Err("密文长度异常".into());
    }
    let salt = &blob[..SALT_LEN];
    let nonce_bytes = &blob[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ct = &blob[SALT_LEN + NONCE_LEN..];

    let key = derive_key(passphrase, salt);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化解密器失败: {}", e))?;
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| "解密失败：同步口令不正确，或密码库已损坏".to_string())?;
    String::from_utf8(pt).map_err(|e| format!("解密内容不是有效文本: {}", e))
}

/// 用已派生好的密钥加密（批量场景复用同一 key）
pub fn encrypt_with_key(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("初始化加密器失败: {}", e))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| format!("加密失败: {}", e))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 用已派生好的密钥解密
pub fn decrypt_with_key(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < NONCE_LEN + 16 {
        return Err("密文长度异常".into());
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("初始化解密器失败: {}", e))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| "解密失败：主密码不正确，或数据已损坏".to_string())
}

/// 从口令 + 固定 salt 派生一次密钥（批量加密）
pub fn derive_batch_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    derive_key(passphrase, salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let plain = r#"{"web-prod":"p@ss word!","db":"中文密码"}"#;
        let enc = encrypt("my-secret-passphrase", plain).unwrap();
        assert_ne!(enc, plain);
        let dec = decrypt("my-secret-passphrase", &enc).unwrap();
        assert_eq!(dec, plain);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let enc = encrypt("right-key", "secret").unwrap();
        assert!(decrypt("wrong-key", &enc).is_err());
    }

    #[test]
    fn batch_key_roundtrip() {
        let salt = [7u8; SALT_LEN];
        let key = derive_batch_key("batch-pass", &salt);
        let plain = b"hello encrypted world";
        let blob = encrypt_with_key(&key, plain).unwrap();
        assert_ne!(&blob[..], plain);
        let dec = decrypt_with_key(&key, &blob).unwrap();
        assert_eq!(dec, plain);
        // 错误 key 应失败
        let wrong = derive_batch_key("wrong", &salt);
        assert!(decrypt_with_key(&wrong, &blob).is_err());
    }

    #[test]
    fn random_nonce_differs() {
        let a = encrypt("k", "same").unwrap();
        let b = encrypt("k", "same").unwrap();
        assert_ne!(a, b, "每次加密应使用不同 nonce");
    }
}
