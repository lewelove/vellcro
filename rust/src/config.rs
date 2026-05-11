use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    pub collection: Option<CollectionConfig>,
}

#[derive(Deserialize)]
pub struct StorageConfig {
    pub cache: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self { cache: None }
    }
}

#[derive(Deserialize, Default)]
pub struct CollectionConfig {
    pub folder: String,
}

impl AppConfig {
    pub fn load() -> Self {
        let config_path = crate::utils::expand_path("~/.config/vellcro/config.toml");
        if let Ok(content) = std::fs::read_to_string(config_path) {
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn get_cache_dir(&self) -> PathBuf {
        self.storage.cache.as_ref()
            .map(|p| crate::utils::expand_path(p))
            .unwrap_or_else(|| crate::utils::expand_path("~/.cache/vellcro"))
    }

    pub fn get_collection_folder(&self) -> Option<PathBuf> {
        self.collection.as_ref().map(|c| crate::utils::expand_path(&c.folder))
    }
}
