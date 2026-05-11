use indexmap::IndexMap;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    pub collection: Option<CollectionConfig>,
    pub manifest: Option<ManifestConfig>,
}

#[derive(Deserialize)]
#[derive(Default)]
pub struct StorageConfig {
    pub cache: Option<String>,
}


#[derive(Deserialize, Default)]
pub struct CollectionConfig {
    pub folder: String,
}

#[derive(Deserialize, Default, Clone)]
pub struct ManifestConfig {
    pub metadata: Option<IndexMap<String, ManifestKeyConfig>>,
    pub mbid: Option<IndexMap<String, ManifestKeyConfig>>,
    pub url: Option<IndexMap<String, ManifestKeyConfig>>,
}

#[derive(Deserialize, Default, Clone)]
pub struct ManifestKeyConfig {
    pub level: String,
    #[serde(default)]
    pub newline: bool,
}

impl AppConfig {
    pub fn load() -> Self {
        let config_path = crate::utils::expand_path("~/.config/vellcro/config.toml");
        std::fs::read_to_string(config_path).map_or_else(
            |_| Self::default(),
            |content| toml::from_str(&content).unwrap_or_default(),
        )
    }

    pub fn get_cache_dir(&self) -> PathBuf {
        self.storage.cache.as_ref().map_or_else(|| crate::utils::expand_path("~/.cache/vellcro"), |p| crate::utils::expand_path(p))
    }

    pub fn get_collection_folder(&self) -> Option<PathBuf> {
        self.collection.as_ref().map(|c| crate::utils::expand_path(&c.folder))
    }
}
