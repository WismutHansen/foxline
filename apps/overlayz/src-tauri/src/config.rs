use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum ThemeProvider {
    Custom {
        listening: String,
        thinking: String,
        talking: String,
    },
    Tinty {
        base16_dir: String,
        #[serde(default)]
        listening_keys: Option<[String; 3]>,
        #[serde(default)]
        thinking_keys: Option<[String; 3]>,
        #[serde(default)]
        talking_keys: Option<[String; 3]>,
        #[serde(default)]
        listening_key: Option<String>,
        #[serde(default)]
        thinking_key: Option<String>,
        #[serde(default)]
        talking_key: Option<String>,
    },
}

impl Default for ThemeProvider {
    fn default() -> Self {
        Self::Custom {
            listening: "#CADCFC".to_string(),
            thinking: "#C8A2FF".to_string(),
            talking: "#A2FFB8".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbThemeConfig {
    #[serde(flatten)]
    pub provider: ThemeProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalShortcut {
    pub modifiers: Vec<String>,
    pub key: String,
}

impl GlobalShortcut {
    pub fn to_accelerator(&self) -> String {
        let mut parts = self.modifiers.clone();
        parts.push(self.key.clone());
        parts.join("+")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardShortcuts {
    pub switch_ui: String,
    pub clear_history: String,
    pub toggle_draggable: String,
}

impl Default for KeyboardShortcuts {
    fn default() -> Self {
        Self {
            switch_ui: "Tab".to_string(),
            clear_history: "Escape".to_string(),
            toggle_draggable: "Meta".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalShortcuts {
    pub toggle_audio: GlobalShortcut,
    pub show_kitt: GlobalShortcut,
    pub show_orb: GlobalShortcut,
    pub show_last_agent: GlobalShortcut,
}

impl Default for GlobalShortcuts {
    fn default() -> Self {
        Self {
            toggle_audio: GlobalShortcut {
                modifiers: vec!["CommandOrControl".to_string(), "Alt".to_string()],
                key: "A".to_string(),
            },
            show_kitt: GlobalShortcut {
                modifiers: vec!["CommandOrControl".to_string(), "Alt".to_string()],
                key: "K".to_string(),
            },
            show_orb: GlobalShortcut {
                modifiers: vec!["CommandOrControl".to_string(), "Alt".to_string()],
                key: "O".to_string(),
            },
            show_last_agent: GlobalShortcut {
                modifiers: vec!["CommandOrControl".to_string(), "Alt".to_string()],
                key: "Space".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantConfig {
    pub name: String,
    pub system_prompt: String,
    pub voice: String,
    #[serde(default = "default_speed")]
    pub speed: f32,
    pub history_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<OrbThemeConfig>,
}

fn default_speed() -> f32 {
    1.0
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            name: "K.I.T.T.".to_string(),
            system_prompt: "You are K.I.T.T., a helpful AI assistant from Knight Rider. Be concise and helpful.".to_string(),
            voice: "af_sky".to_string(),
            speed: 1.0,
            history_enabled: true,
            theme: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoxlineConfig {
    #[serde(default = "default_foxline_url")]
    pub url: String,
    #[serde(default = "default_foxline_agent")]
    pub agent: String,
    /// Persona defaults to the agent when empty (gateway protocol default).
    #[serde(default)]
    pub persona: String,
    /// Workspace defaults to `agents/<agent>` when empty.
    #[serde(default)]
    pub workspace: String,
    #[serde(default = "default_foxline_loadout")]
    pub loadout: String,
}

fn default_foxline_url() -> String {
    "ws://127.0.0.1:8780".to_string()
}

fn default_foxline_agent() -> String {
    "campbell".to_string()
}

fn default_foxline_loadout() -> String {
    "default".to_string()
}

impl Default for FoxlineConfig {
    fn default() -> Self {
        Self {
            url: default_foxline_url(),
            agent: default_foxline_agent(),
            persona: String::new(),
            workspace: String::new(),
            loadout: default_foxline_loadout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub history_path: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self { history_path: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub kitt: AssistantConfig,
    pub orb: AssistantConfig,
    pub keyboard: KeyboardShortcuts,
    pub global_shortcuts: GlobalShortcuts,
    pub storage: StorageConfig,
    #[serde(default)]
    pub foxline: FoxlineConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut kitt = AssistantConfig::default();
        kitt.name = "K.I.T.T.".to_string();
        kitt.system_prompt = "You are K.I.T.T., the Knight Industries Two Thousand. You are a sophisticated AI in a Trans Am. Be helpful, professional, and occasionally witty like the original character.".to_string();
        kitt.voice = "am_michael".to_string();
        kitt.speed = 1.0;

        let mut orb = AssistantConfig::default();
        orb.name = "Orb".to_string();
        orb.system_prompt =
            "You are Orb, a modern AI assistant. Be friendly, helpful, and conversational."
                .to_string();
        orb.voice = "af_sky".to_string();
        orb.speed = 1.0;
        orb.theme = Some(OrbThemeConfig {
            provider: ThemeProvider::Custom {
                listening: "#CADCFC".to_string(),
                thinking: "#C8A2FF".to_string(),
                talking: "#A2FFB8".to_string(),
            },
        });

        Self {
            kitt,
            orb,
            keyboard: KeyboardShortcuts::default(),
            global_shortcuts: GlobalShortcuts::default(),
            storage: StorageConfig::default(),
            foxline: FoxlineConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config).join("overlayz")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config").join("overlayz")
        }
    }

    pub fn data_dir() -> PathBuf {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            PathBuf::from(xdg_data).join("overlayz")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("overlayz")
        }
    }

    pub fn state_dir() -> PathBuf {
        if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
            PathBuf::from(xdg_state).join("overlayz")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("overlayz")
        }
    }

    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir();
        let config_path = config_dir.join("config.toml");

        if !config_path.exists() {
            let default_config = Self::default();
            fs::create_dir_all(&config_dir)?;
            let toml_str = toml::to_string_pretty(&default_config)?;
            fs::write(&config_path, toml_str)?;
            println!(
                "[Config] Created default config at: {}",
                config_path.display()
            );
            return Ok(default_config);
        }

        let config_str = fs::read_to_string(&config_path)?;
        let mut config: AppConfig = toml::from_str(&config_str)
            .map_err(|e| format!("Failed to parse config at {}: {}", config_path.display(), e))?;

        if config.storage.history_path.is_none() {
            let data_dir = Self::data_dir();
            fs::create_dir_all(&data_dir)?;
            config.storage.history_path =
                Some(data_dir.join("history").to_string_lossy().to_string());
        }

        Ok(config)
    }
}
