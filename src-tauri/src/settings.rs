use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    version: u8,
    sticker_library_root: Option<PathBuf>,
}

pub struct AppSettings {
    settings_path: PathBuf,
    default_sticker_library_root: PathBuf,
    sticker_library_root: PathBuf,
}

impl AppSettings {
    pub fn initialize(
        settings_path: PathBuf,
        default_sticker_library_root: PathBuf,
        legacy_settings_path: Option<PathBuf>,
    ) -> Result<Self> {
        let persisted = read_settings(&settings_path)
            .or_else(|| legacy_settings_path.as_deref().and_then(read_settings));
        let sticker_library_root = persisted
            .and_then(|settings| settings.sticker_library_root)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| default_sticker_library_root.clone());
        let settings = Self {
            settings_path,
            default_sticker_library_root,
            sticker_library_root,
        };
        if !settings.settings_path.exists() {
            settings.persist()?;
        }
        Ok(settings)
    }

    pub fn sticker_library_root(&self) -> &Path {
        &self.sticker_library_root
    }

    pub fn default_sticker_library_root(&self) -> &Path {
        &self.default_sticker_library_root
    }

    pub fn set_sticker_library_root(&mut self, root_directory: PathBuf) -> Result<()> {
        if !root_directory.is_absolute() {
            return Err(anyhow!("表情库目录必须是绝对路径"));
        }
        let previous = self.sticker_library_root.clone();
        self.sticker_library_root = root_directory;
        if let Err(error) = self.persist() {
            self.sticker_library_root = previous;
            return Err(error);
        }
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = PersistedSettings {
            version: 1,
            sticker_library_root: Some(self.sticker_library_root.clone()),
        };
        let mut bytes = serde_json::to_vec_pretty(&payload)?;
        bytes.push(b'\n');
        let temporary = self.settings_path.with_extension("json.tmp");
        fs::write(&temporary, bytes)?;
        if self.settings_path.exists() {
            fs::remove_file(&self.settings_path)?;
        }
        fs::rename(temporary, &self.settings_path)?;
        Ok(())
    }
}

fn read_settings(path: &Path) -> Option<PersistedSettings> {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
}
