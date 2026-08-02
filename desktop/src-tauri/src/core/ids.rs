use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::core::error::{CoreError, CoreResult};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct ProjectSecret([u8; 32]);

impl ProjectSecret {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub trait ProjectSecretProvider: Send + Sync {
    fn secret_for(&self, project_id: &str) -> CoreResult<ProjectSecret>;
}

#[derive(Clone)]
pub struct FixedSecretProvider {
    secret: ProjectSecret,
}

impl FixedSecretProvider {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            secret: ProjectSecret::from_bytes(bytes),
        }
    }
}

impl ProjectSecretProvider for FixedSecretProvider {
    fn secret_for(&self, _: &str) -> CoreResult<ProjectSecret> {
        Ok(self.secret.clone())
    }
}

/// The production provider is deliberately isolated from the project work directory.
/// It uses the operating-system credential store and never serializes a secret into SQLite.
pub struct SystemCredentialSecretProvider {
    store: Arc<dyn CredentialStore>,
}

trait CredentialStore: Send + Sync {
    fn read(&self, project_id: &str) -> Result<String, CredentialStoreError>;
    fn write(&self, project_id: &str, encoded_secret: &str) -> Result<(), CredentialStoreError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CredentialStoreError {
    NoEntry,
    Unavailable,
}

struct KeyringCredentialStore;

impl CredentialStore for KeyringCredentialStore {
    fn read(&self, project_id: &str) -> Result<String, CredentialStoreError> {
        let entry = keyring::Entry::new("com.local.gwpannotator.project-secret", project_id)
            .map_err(|_| CredentialStoreError::Unavailable)?;
        match entry.get_password() {
            Ok(encoded) => Ok(encoded),
            Err(keyring::Error::NoEntry) => Err(CredentialStoreError::NoEntry),
            Err(_) => Err(CredentialStoreError::Unavailable),
        }
    }

    fn write(&self, project_id: &str, encoded_secret: &str) -> Result<(), CredentialStoreError> {
        let entry = keyring::Entry::new("com.local.gwpannotator.project-secret", project_id)
            .map_err(|_| CredentialStoreError::Unavailable)?;
        entry
            .set_password(encoded_secret)
            .map_err(|_| CredentialStoreError::Unavailable)
    }
}

impl Default for SystemCredentialSecretProvider {
    fn default() -> Self {
        Self {
            store: Arc::new(KeyringCredentialStore),
        }
    }
}

impl ProjectSecretProvider for SystemCredentialSecretProvider {
    fn secret_for(&self, project_id: &str) -> CoreResult<ProjectSecret> {
        match self.store.read(project_id) {
            Ok(encoded) => decode_secret(&encoded),
            Err(CredentialStoreError::NoEntry) => {
                let mut bytes = [0_u8; 32];
                OsRng.fill_bytes(&mut bytes);
                self.store
                    .write(project_id, &BASE64_STANDARD.encode(bytes))
                    .map_err(|_| CoreError::CredentialStoreUnavailable)?;
                Ok(ProjectSecret::from_bytes(bytes))
            }
            Err(CredentialStoreError::Unavailable) => Err(CoreError::CredentialStoreUnavailable),
        }
    }
}

pub type SharedSecretProvider = Arc<dyn ProjectSecretProvider>;

pub fn campus_code(secret: &ProjectSecret, raw_campus_name: &str) -> String {
    opaque_hmac_id("CAMPUS-", secret, &[raw_campus_name.as_bytes()])
}

pub fn case_id(secret: &ProjectSecret, campus_code: &str, raw_case_name: &str) -> String {
    opaque_hmac_id(
        "CASE-",
        secret,
        &[campus_code.as_bytes(), b"\0", raw_case_name.as_bytes()],
    )
}

pub fn asset_id(
    secret: &ProjectSecret,
    case_id: &str,
    raw_media_name: &str,
    size: u64,
    modified_millis: u128,
) -> String {
    opaque_hmac_id(
        "ASSET-",
        secret,
        &[
            case_id.as_bytes(),
            b"\0",
            raw_media_name.as_bytes(),
            b"\0",
            size.to_string().as_bytes(),
            b"\0",
            modified_millis.to_string().as_bytes(),
        ],
    )
}

pub fn source_fingerprint(
    secret: &ProjectSecret,
    case_id: &str,
    raw_media_name: &str,
    size: u64,
    modified_millis: u128,
) -> String {
    opaque_hmac_id(
        "FP-",
        secret,
        &[
            case_id.as_bytes(),
            b"\0",
            raw_media_name.as_bytes(),
            b"\0",
            size.to_string().as_bytes(),
            b"\0",
            modified_millis.to_string().as_bytes(),
        ],
    )
}

pub fn stable_template_component(prefix: &str, text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let encoded = BASE32_NOPAD.encode(digest.as_slice());
    format!("{prefix}-{}", &encoded[..12])
}

pub fn is_opaque_id(value: &str, prefix: &str) -> bool {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return false;
    };
    suffix.len() >= 8
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || matches!(byte, b'2'..=b'7'))
}

fn opaque_hmac_id(prefix: &str, secret: &ProjectSecret, parts: &[&[u8]]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.bytes()).expect("HMAC accepts a fixed-size key");
    for part in parts {
        mac.update(part);
    }
    let digest = mac.finalize().into_bytes();
    let encoded = BASE32_NOPAD.encode(&digest);
    format!("{prefix}{}", &encoded[..16])
}

fn decode_secret(encoded: &str) -> CoreResult<ProjectSecret> {
    let decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| CoreError::InvalidSecret)?;
    let bytes: [u8; 32] = decoded.try_into().map_err(|_| CoreError::InvalidSecret)?;
    Ok(ProjectSecret::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::{
        asset_id, case_id, is_opaque_id, CredentialStore, CredentialStoreError,
        FixedSecretProvider, ProjectSecretProvider, SystemCredentialSecretProvider,
    };
    use crate::core::error::CoreError;

    struct FailingCredentialStore {
        writes: Arc<AtomicUsize>,
        failure: CredentialStoreError,
    }

    impl CredentialStore for FailingCredentialStore {
        fn read(&self, _: &str) -> Result<String, CredentialStoreError> {
            Err(self.failure)
        }

        fn write(&self, _: &str, _: &str) -> Result<(), CredentialStoreError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn opaque_ids_are_stable_and_do_not_embed_input() {
        let provider = FixedSecretProvider::from_bytes([7; 32]);
        let secret = provider.secret_for("PROJECT-LOCAL").expect("fixed secret");
        let case = case_id(&secret, "CAMPUS-ABCDEFGH", "20240101_CASE-ALPHA");
        let asset = asset_id(&secret, &case, "ASSET-01.JPG", 12, 99);

        assert!(is_opaque_id(&case, "CASE-"));
        assert!(is_opaque_id(&asset, "ASSET-"));
        assert!(!case.contains("20240101"));
        assert!(!asset.contains("ASSET-01"));
    }

    #[test]
    fn credential_store_errors_never_generate_or_replace_a_secret() {
        let writes = Arc::new(AtomicUsize::new(0));
        let provider = SystemCredentialSecretProvider {
            store: Arc::new(FailingCredentialStore {
                writes: Arc::clone(&writes),
                failure: CredentialStoreError::Unavailable,
            }),
        };

        let result = provider.secret_for("PROJECT-LOCAL");
        assert!(matches!(result, Err(CoreError::CredentialStoreUnavailable)));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn only_an_explicit_missing_entry_can_generate_a_secret() {
        let writes = Arc::new(AtomicUsize::new(0));
        let provider = SystemCredentialSecretProvider {
            store: Arc::new(FailingCredentialStore {
                writes: Arc::clone(&writes),
                failure: CredentialStoreError::NoEntry,
            }),
        };

        assert!(provider.secret_for("PROJECT-LOCAL").is_ok());
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}
