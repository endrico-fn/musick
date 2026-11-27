use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use tracing::debug;

use crate::error::{CoreError, CoreResult};
use crate::paths::{AppPaths, atomic_write_text};

const KEYRING_SERVICE: &str = "musick";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> CoreResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> CoreResult<()>;
    fn delete(&self, key: &str) -> CoreResult<()>;
}

#[derive(Debug, Default)]
pub struct MemorySecretStore {
    data: RwLock<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        let guard = self
            .data
            .read()
            .map_err(|_| CoreError::SecretStore("MemorySecretStore lock poisoned".into()))?;
        Ok(guard.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> CoreResult<()> {
        let mut guard = self
            .data
            .write()
            .map_err(|_| CoreError::SecretStore("MemorySecretStore lock poisoned".into()))?;
        guard.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        let mut guard = self
            .data
            .write()
            .map_err(|_| CoreError::SecretStore("MemorySecretStore lock poisoned".into()))?;
        guard.remove(key);
        Ok(())
    }
}

/// os keyring store with atomic `0o600` json file fallback when secret service is unavailable
pub struct KeyringSecretStore {
    fallback_path: PathBuf,
    file_lock: Mutex<()>,
    prefer_keyring: bool,
}

impl KeyringSecretStore {
    pub fn new(paths: &AppPaths) -> Self {
        Self {
            fallback_path: paths.secrets_fallback_file(),
            file_lock: Mutex::new(()),
            prefer_keyring: true,
        }
    }

    pub fn file_only(fallback_path: PathBuf) -> Self {
        Self {
            fallback_path,
            file_lock: Mutex::new(()),
            prefer_keyring: false,
        }
    }

    fn read_fallback_map(&self) -> CoreResult<HashMap<String, String>> {
        match fs::read_to_string(&self.fallback_path) {
            Ok(raw) => {
                if raw.trim().is_empty() {
                    return Ok(HashMap::new());
                }
                Ok(serde_json::from_str::<HashMap<String, String>>(&raw).unwrap_or_default())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(err) => Err(CoreError::Io(err)),
        }
    }

    fn write_fallback_map(&self, map: &HashMap<String, String>) -> CoreResult<()> {
        if map.is_empty() {
            if self.fallback_path.exists() {
                let _ = fs::remove_file(&self.fallback_path);
            }
            return Ok(());
        }
        let encoded = serde_json::to_string_pretty(map)?;
        atomic_write_text(&self.fallback_path, &encoded)
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> CoreResult<Option<String>> {
        if self.prefer_keyring {
            match keyring::Entry::new(KEYRING_SERVICE, key) {
                Ok(entry) => match entry.get_password() {
                    Ok(val) => return Ok(Some(val)),
                    Err(keyring::Error::NoEntry) => {}
                    Err(err) => {
                        debug!("OS keyring read fallback for '{key}': {err}");
                    }
                },
                Err(err) => {
                    debug!("OS keyring unavailable on read for '{key}': {err}");
                }
            }
        }

        let _guard = self
            .file_lock
            .lock()
            .map_err(|_| CoreError::SecretStore("Fallback file lock poisoned".into()))?;
        let map = self.read_fallback_map()?;
        Ok(map.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> CoreResult<()> {
        if self.prefer_keyring {
            match keyring::Entry::new(KEYRING_SERVICE, key) {
                Ok(entry) => {
                    if entry.set_password(value).is_ok() {
                        // Keep fallback file clean if keyring succeeded.
                        if let Ok(_guard) = self.file_lock.lock()
                            && let Ok(mut map) = self.read_fallback_map()
                            && map.remove(key).is_some()
                        {
                            let _ = self.write_fallback_map(&map);
                        }
                        return Ok(());
                    }
                }
                Err(err) => {
                    debug!("OS keyring unavailable on write for '{key}': {err}");
                }
            }
        }

        let _guard = self
            .file_lock
            .lock()
            .map_err(|_| CoreError::SecretStore("Fallback file lock poisoned".into()))?;
        let mut map = self.read_fallback_map()?;
        map.insert(key.to_string(), value.to_string());
        self.write_fallback_map(&map)
    }

    fn delete(&self, key: &str) -> CoreResult<()> {
        if self.prefer_keyring
            && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key)
        {
            let _ = entry.delete_credential();
        }

        let _guard = self
            .file_lock
            .lock()
            .map_err(|_| CoreError::SecretStore("Fallback file lock poisoned".into()))?;
        let mut map = self.read_fallback_map()?;
        if map.remove(key).is_some() {
            self.write_fallback_map(&map)?;
        }
        Ok(())
    }
}
