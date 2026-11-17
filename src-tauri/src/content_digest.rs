use std::sync::{Arc, RwLock};

use hmac::{Hmac, Mac};
use keyring::Entry;
use rand::RngCore;
use sha2::Sha256;

const SERVICE: &str = "com.dshanpi.lynx.audit";
const ACCOUNT: &str = "content-hmac-v1";

pub trait ContentDigest: Send + Sync {
    fn digest(&self, frames: &[&[u8]]) -> Result<String, String>;
}

#[derive(Default)]
pub struct ContentDigestState {
    override_digest: RwLock<Option<Arc<dyn ContentDigest>>>,
}

impl ContentDigestState {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn replace_for_test(&self, digest: Arc<dyn ContentDigest>) {
        *self.override_digest.write().unwrap() = Some(digest);
    }

    fn digest(&self, frames: &[&[u8]]) -> Result<String, String> {
        if let Some(digest) = self
            .override_digest
            .read()
            .ok()
            .and_then(|value| value.clone())
        {
            return digest.digest(frames);
        }
        digest_with_key(&installation_key()?, frames)
    }
}

pub fn digest(app: &tauri::AppHandle, frames: &[&[u8]]) -> Result<String, String> {
    use tauri::Manager;
    app.state::<ContentDigestState>().digest(frames)
}

fn installation_key() -> Result<Vec<u8>, String> {
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(|_| "CONTENT_DIGEST_KEY_UNAVAILABLE")?;
    match entry.get_password() {
        Ok(encoded) => hex::decode(encoded).map_err(|_| "CONTENT_DIGEST_KEY_INVALID".to_owned()),
        Err(keyring::Error::NoEntry) => {
            let mut key = [0_u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            entry
                .set_password(&hex::encode(key))
                .map_err(|_| "CONTENT_DIGEST_KEY_SAVE_FAILED".to_owned())?;
            Ok(key.to_vec())
        }
        Err(_) => Err("CONTENT_DIGEST_KEY_UNAVAILABLE".to_owned()),
    }
}

fn digest_with_key(key: &[u8], frames: &[&[u8]]) -> Result<String, String> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| "CONTENT_DIGEST_KEY_INVALID".to_owned())?;
    for frame in frames {
        mac.update(&(frame.len() as u64).to_be_bytes());
        mac.update(frame);
    }
    Ok(hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_length_framed() {
        let key = [7_u8; 32];
        let first = digest_with_key(&key, &[b"ab", b"c"]).unwrap();
        assert_eq!(first, digest_with_key(&key, &[b"ab", b"c"]).unwrap());
        assert_ne!(first, digest_with_key(&key, &[b"a", b"bc"]).unwrap());
        assert_eq!(first.len(), 64);
    }
}
