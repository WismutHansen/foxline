use crate::config::{AppConfig, ThemeProvider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Base16Theme {
    pub system: String,
    pub name: String,
    pub author: String,
    pub variant: String,
    pub palette: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbColors {
    pub listening: [String; 3],
    pub thinking: [String; 3],
    pub talking: [String; 3],
}

pub struct ThemeManager;

impl ThemeManager {
    pub fn get_current_theme() -> Result<String, Box<dyn std::error::Error>> {
        let state_dir = AppConfig::state_dir();
        let theme_file = state_dir.join("current_theme.txt");

        if theme_file.exists() {
            let content = fs::read_to_string(&theme_file)?;
            Ok(content.trim().to_string())
        } else {
            Ok("default".to_string())
        }
    }

    pub fn set_current_theme(theme_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let state_dir = AppConfig::state_dir();
        fs::create_dir_all(&state_dir)?;

        let theme_file = state_dir.join("current_theme.txt");
        fs::write(&theme_file, theme_name)?;

        Ok(())
    }

    pub fn normalize_theme_name(name: &str) -> String {
        if name.starts_with("base16-") {
            name.strip_prefix("base16-").unwrap().to_string()
        } else {
            name.to_string()
        }
    }

    pub fn find_theme_file(base_dir: &Path, theme_name: &str) -> Option<PathBuf> {
        let normalized = Self::normalize_theme_name(theme_name);

        let patterns = vec![
            format!("base16-{}.yaml", normalized),
            format!("{}.yaml", normalized),
            format!("base16-{}.yml", normalized),
            format!("{}.yml", normalized),
        ];

        for pattern in patterns {
            let path = base_dir.join(&pattern);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    pub fn load_base16_theme(theme_path: &Path) -> Result<Base16Theme, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(theme_path)?;
        let theme: Base16Theme = serde_yaml::from_str(&content)?;
        Ok(theme)
    }

    pub fn derive_gradient_colors(base_color: &str) -> [String; 3] {
        let color = Self::hex_to_hsl(base_color);

        let (h, s, l) = color;

        let color1 = Self::hsl_to_hex(h, s, l);
        let color2 = Self::hsl_to_hex(h, (s * 0.85).min(100.0), (l * 0.92).max(0.0));
        let color3 = Self::hsl_to_hex(h, (s * 0.75).min(100.0), (l * 0.85).max(0.0));

        [color1, color2, color3]
    }

    pub fn get_orb_colors(
        provider: &ThemeProvider,
    ) -> Result<OrbColors, Box<dyn std::error::Error>> {
        match provider {
            ThemeProvider::Custom {
                listening,
                thinking,
                talking,
            } => Ok(OrbColors {
                listening: Self::derive_gradient_colors(listening),
                thinking: Self::derive_gradient_colors(thinking),
                talking: Self::derive_gradient_colors(talking),
            }),
            ThemeProvider::Tinty {
                base16_dir,
                listening_keys,
                thinking_keys,
                talking_keys,
                listening_key,
                thinking_key,
                talking_key,
            } => {
                let theme_name =
                    Self::get_current_theme().unwrap_or_else(|_| "default".to_string());

                let expanded_dir = shellexpand::tilde(base16_dir);
                let base_dir = Path::new(expanded_dir.as_ref());

                let theme_file = Self::find_theme_file(base_dir, &theme_name)
                    .ok_or_else(|| format!("Theme '{}' not found in {}", theme_name, base16_dir))?;

                let theme = Self::load_base16_theme(&theme_file)?;

                let get_color = |key: &str| -> Result<String, Box<dyn std::error::Error>> {
                    theme
                        .palette
                        .get(key)
                        .ok_or_else(|| format!("Color key '{}' not found in theme", key).into())
                        .map(|s| s.clone())
                };

                // Helper to get colors either from single key (with gradient) or three keys
                let get_state_colors =
                    |single_key: &Option<String>,
                     triple_keys: &Option<[String; 3]>,
                     state_name: &str|
                     -> Result<[String; 3], Box<dyn std::error::Error>> {
                        if let Some(key) = single_key {
                            // Single key mode: generate gradient
                            let base_color = get_color(key)?;
                            Ok(Self::derive_gradient_colors(&base_color))
                        } else if let Some(keys) = triple_keys {
                            // Triple key mode: use exact colors
                            Ok([
                                get_color(&keys[0])?,
                                get_color(&keys[1])?,
                                get_color(&keys[2])?,
                            ])
                        } else {
                            Err(format!(
                                "Must specify either {}_key or {}_keys in tinty configuration",
                                state_name, state_name
                            )
                            .into())
                        }
                    };

                let listening_colors =
                    get_state_colors(listening_key, listening_keys, "listening")?;
                let thinking_colors = get_state_colors(thinking_key, thinking_keys, "thinking")?;
                let talking_colors = get_state_colors(talking_key, talking_keys, "talking")?;

                Ok(OrbColors {
                    listening: listening_colors,
                    thinking: thinking_colors,
                    talking: talking_colors,
                })
            }
        }
    }

    fn hex_to_hsl(hex: &str) -> (f32, f32, f32) {
        let hex = hex.trim_start_matches('#');

        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let l = (max + min) / 2.0;

        if delta == 0.0 {
            return (0.0, 0.0, l * 100.0);
        }

        let s = if l < 0.5 {
            delta / (max + min)
        } else {
            delta / (2.0 - max - min)
        };

        let h = if max == r {
            ((g - b) / delta + if g < b { 6.0 } else { 0.0 }) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };

        (h * 360.0, s * 100.0, l * 100.0)
    }

    fn hsl_to_hex(h: f32, s: f32, l: f32) -> String {
        let h = h / 360.0;
        let s = s / 100.0;
        let l = l / 100.0;

        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;

        let (r, g, b) = if h < 1.0 / 6.0 {
            (c, x, 0.0)
        } else if h < 2.0 / 6.0 {
            (x, c, 0.0)
        } else if h < 3.0 / 6.0 {
            (0.0, c, x)
        } else if h < 4.0 / 6.0 {
            (0.0, x, c)
        } else if h < 5.0 / 6.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        let r = ((r + m) * 255.0) as u8;
        let g = ((g + m) * 255.0) as u8;
        let b = ((b + m) * 255.0) as u8;

        format!("#{:02X}{:02X}{:02X}", r, g, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_theme_name() {
        assert_eq!(ThemeManager::normalize_theme_name("base16-nord"), "nord");
        assert_eq!(ThemeManager::normalize_theme_name("nord"), "nord");
    }

    #[test]
    fn test_hex_to_hsl_conversion() {
        let (h, _s, _l) = ThemeManager::hex_to_hsl("#CADCFC");
        assert!((h - 218.0).abs() < 5.0);
    }

    #[test]
    fn test_derive_gradient_colors() {
        let colors = ThemeManager::derive_gradient_colors("#CADCFC");
        assert_eq!(colors.len(), 3);
        assert!(colors[0].starts_with('#'));
    }
}
