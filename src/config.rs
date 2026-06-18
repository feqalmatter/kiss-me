use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_active_profile")]
    pub active_profile: String,
    #[serde(default)]
    pub nexus_api_key: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub game_source: PathBuf,
    pub library: PathBuf,
    pub downloads: PathBuf,
    pub output: PathBuf,
    pub manifest: PathBuf,
    #[serde(default)]
    pub nexus_game_domain: Option<String>,
}

fn default_active_profile() -> String {
    "default".to_string()
}

impl Config {
    pub fn path(explicit: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = explicit {
            return Ok(path.to_path_buf());
        }

        if let Ok(path) = env::var("KISS_ME_CONFIG") {
            if !path.trim().is_empty() {
                return Ok(PathBuf::from(path));
            }
        }

        let local = env::current_dir()?.join("kiss-me.toml");
        if local.exists() {
            return Ok(local);
        }

        let home = home_dir()?;
        Ok(home.join(".config").join("kiss-me").join("config.toml"))
    }

    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = Self::path(explicit)?;
        let mut config = if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config: {}", path.display()))?;
            toml::from_str(&text)
                .with_context(|| format!("failed to parse config: {}", path.display()))?
        } else {
            Self::default_for_cwd()?
        };

        if config.profiles.is_empty() {
            config
                .profiles
                .insert(default_active_profile(), Profile::default_for_cwd()?);
        }

        Ok(config)
    }

    pub fn write_starter(explicit: Option<&Path>) -> Result<PathBuf> {
        let path = Self::path(explicit)?;
        if path.exists() {
            bail!("config already exists: {}", path.display());
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory: {}", parent.display())
            })?;
        }

        let text = toml::to_string_pretty(&Self::default_for_cwd()?)?;
        fs::write(&path, text)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(path)
    }

    pub fn active_profile(&self, override_name: Option<&str>) -> Result<Profile> {
        let name = override_name.unwrap_or(&self.active_profile);
        let mut profile = self
            .profiles
            .get(name)
            .cloned()
            .with_context(|| format!("unknown profile: {name}"))?;
        profile.apply_env_overrides();
        profile.expand_home()?;
        Ok(profile)
    }

    pub fn nexus_api_key(&self) -> Option<String> {
        self.nexus_api_key
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn default_for_cwd() -> Result<Self> {
        let mut profiles = BTreeMap::new();
        profiles.insert(default_active_profile(), Profile::default_for_cwd()?);
        Ok(Self {
            active_profile: default_active_profile(),
            nexus_api_key: None,
            profiles,
        })
    }
}

impl Profile {
    fn default_for_cwd() -> Result<Self> {
        let cwd = env::current_dir()?;
        let home = home_dir()?;
        let game_source = home.join("Games").join("Heroic").join("Cyberpunk 2077");
        Ok(Self {
            output: PathBuf::from(format!("{}.modded", game_source.display())),
            game_source,
            library: cwd.join("Library"),
            downloads: cwd.join("Downloads"),
            manifest: cwd.join(".manifest"),
            nexus_game_domain: Some("cyberpunk2077".to_string()),
        })
    }

    fn apply_env_overrides(&mut self) {
        apply_path_env("GAME_SOURCE", &mut self.game_source);
        apply_path_env("LIBRARY", &mut self.library);
        apply_path_env("DOWNLOADS", &mut self.downloads);
        apply_path_env("OUTPUT", &mut self.output);
        apply_path_env("MANIFEST", &mut self.manifest);
        if let Ok(value) = env::var("NEXUS_GAME_DOMAIN") {
            if !value.trim().is_empty() {
                self.nexus_game_domain = Some(value);
            }
        }
    }

    fn expand_home(&mut self) -> Result<()> {
        self.game_source = expand_home(&self.game_source)?;
        self.library = expand_home(&self.library)?;
        self.downloads = expand_home(&self.downloads)?;
        self.output = expand_home(&self.output)?;
        self.manifest = expand_home(&self.manifest)?;
        Ok(())
    }
}

fn apply_path_env(name: &str, path: &mut PathBuf) {
    if let Ok(value) = env::var(name) {
        if !value.trim().is_empty() {
            *path = PathBuf::from(value);
        }
    }
}

fn expand_home(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" {
        return home_dir();
    }

    if let Some(rest) = text.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }

    Ok(path.to_path_buf())
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context("HOME is not set")
}
