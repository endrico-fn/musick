use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{CoreError, CoreResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl AppPaths {
    pub fn default_system() -> CoreResult<Self> {
        let proj = ProjectDirs::from("io", "musick", "musick");

        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("musick"))
            .or_else(|| proj.as_ref().map(|p| p.config_dir().to_path_buf()))
            .ok_or_else(|| CoreError::Config("Unable to resolve config directory".into()))?;

        let data_dir = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("musick"))
            .or_else(|| proj.as_ref().map(|p| p.data_dir().to_path_buf()))
            .ok_or_else(|| CoreError::Config("Unable to resolve data directory".into()))?;

        let cache_dir = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("musick"))
            .or_else(|| proj.as_ref().map(|p| p.cache_dir().to_path_buf()))
            .ok_or_else(|| CoreError::Config("Unable to resolve cache directory".into()))?;

        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(|p| PathBuf::from(p).join("musick"))
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("musick-rt-{}", std::process::id()))
            });

        Ok(Self {
            config_dir,
            data_dir,
            cache_dir,
            runtime_dir,
        })
    }

    pub fn from_roots(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        runtime_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            cache_dir: cache_dir.into(),
            runtime_dir: runtime_dir.into(),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn user_db_file(&self) -> PathBuf {
        self.data_dir.join("musick.db")
    }

    pub fn secrets_fallback_file(&self) -> PathBuf {
        self.data_dir.join("credentials.json")
    }

    pub fn cache_db_file(&self) -> PathBuf {
        self.cache_dir.join("cache.db")
    }

    pub fn images_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("images")
    }

    pub fn rustypipe_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("rustypipe")
    }

    pub fn mpv_socket_path(&self) -> PathBuf {
        self.runtime_dir
            .join(format!("mpv-{}.sock", std::process::id()))
    }

    pub fn ensure_dirs(&self) -> CoreResult<()> {
        ensure_private_dir(&self.config_dir)?;
        ensure_private_dir(&self.data_dir)?;
        ensure_private_dir(&self.cache_dir)?;
        ensure_private_dir(&self.images_cache_dir())?;
        ensure_private_dir(&self.rustypipe_cache_dir())?;
        ensure_private_dir(&self.runtime_dir)?;
        Ok(())
    }
}

pub fn ensure_private_dir(path: &Path) -> CoreResult<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp_name = format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    );
    let tmp_path = parent.join(tmp_name);

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let write_res = (|| -> CoreResult<()> {
        let mut file = opts.open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(CoreError::Io(err));
    }

    Ok(())
}

pub fn atomic_write_text(path: &Path, text: &str) -> CoreResult<()> {
    atomic_write_bytes(path, text.as_bytes())
}
