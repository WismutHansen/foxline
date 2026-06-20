use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::LoadoutConfig;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceLoadout {
    pub pi: PiLoadout,
    pub lifecycle: LifecycleLoadout,
    pub adapters: AdapterLoadout,
    pub tools: ToolLoadout,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default)]
pub struct PiLoadout {
    pub profile: Option<String>,
    pub config: Option<String>,
    pub session_dir: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub append_system_prompt: Option<String>,
    pub append_system_prompt_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct LifecycleLoadout {
    pub prewarm: bool,
    pub keep_warm_ms: u64,
}

impl Default for LifecycleLoadout {
    fn default() -> Self {
        Self {
            prewarm: false,
            keep_warm_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct AdapterLoadout {
    pub stt: String,
    pub tts: String,
}

impl Default for AdapterLoadout {
    fn default() -> Self {
        Self {
            stt: "parakeet-silero".to_string(),
            tts: "qwen3-worker".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(default)]
pub struct ToolLoadout {
    pub required_frontend: Vec<String>,
    pub optional_frontend: Vec<String>,
    pub allowed_pi: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLoadout {
    pub name: String,
    pub workspace: PathBuf,
    pub source: Option<PathBuf>,
    pub loadout: VoiceLoadout,
    pub extension_paths: Vec<PathBuf>,
}

pub struct LoadoutResolver {
    config: LoadoutConfig,
}

impl LoadoutResolver {
    pub fn new(config: LoadoutConfig) -> Self {
        Self { config }
    }

    pub fn resolve(
        &self,
        workspace: impl AsRef<Path>,
        name: Option<&str>,
    ) -> Result<ResolvedLoadout> {
        let workspace = workspace.as_ref().to_path_buf();
        let name = name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&self.config.default_name)
            .to_string();
        let source = find_loadout_file(&workspace, &name);
        let loadout = match &source {
            Some(path) => {
                let contents = std::fs::read_to_string(path)
                    .with_context(|| format!("read voice loadout {}", path.display()))?;
                toml::from_str::<VoiceLoadout>(&contents)
                    .with_context(|| format!("parse voice loadout {}", path.display()))?
            }
            None => VoiceLoadout::default(),
        };
        let extension_paths = self.resolve_extensions(&workspace, &loadout.extensions)?;

        Ok(ResolvedLoadout {
            name,
            workspace,
            source,
            loadout,
            extension_paths,
        })
    }

    fn resolve_extensions(&self, workspace: &Path, refs: &[String]) -> Result<Vec<PathBuf>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for reference in refs {
            let path = self.resolve_extension(workspace, reference)?;
            if seen.insert(path.clone()) {
                out.push(path);
            }
        }
        Ok(out)
    }

    fn resolve_extension(&self, workspace: &Path, reference: &str) -> Result<PathBuf> {
        if let Some(name) = reference.strip_prefix("builtin:") {
            ensure_safe_name(name, reference)?;
            let path = self.config.bundled_extensions_dir.join(name);
            ensure_dir(&path, reference)?;
            return Ok(path);
        }

        let local_root = workspace.join(".foxline").join("extensions");
        let path = if let Some(name) = reference.strip_prefix("local:") {
            ensure_safe_name(name, reference)?;
            local_root.join(name)
        } else {
            let raw = PathBuf::from(reference);
            if raw.is_absolute() {
                bail!("voice extension reference must not be absolute: {reference}");
            }
            workspace.join(".foxline").join(raw)
        };

        ensure_dir(&path, reference)?;
        Ok(path)
    }
}

fn find_loadout_file(workspace: &Path, name: &str) -> Option<PathBuf> {
    let foxline = workspace.join(".foxline");
    let named = foxline.join("loadouts").join(format!("{name}.toml"));
    if named.exists() {
        return Some(named);
    }
    let default = foxline.join("loadout.toml");
    if name == "default" && default.exists() {
        return Some(default);
    }
    None
}

fn ensure_safe_name(name: &str, reference: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        bail!("invalid voice extension reference: {reference}");
    }
    Ok(())
}

fn ensure_dir(path: &Path, reference: &str) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "voice extension reference {reference} resolved to missing directory {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::config::LoadoutConfig;

    use super::LoadoutResolver;

    #[test]
    fn resolves_default_loadout_with_builtin_and_local_extensions() {
        let dir = tempdir().unwrap();
        let bundled = dir.path().join("bundled");
        let builtin = bundled.join("frontend-tools");
        let local = dir.path().join(".foxline/extensions/project-tools");
        fs::create_dir_all(&builtin).unwrap();
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(dir.path().join(".foxline/loadouts")).unwrap();
        fs::write(
            dir.path().join(".foxline/loadouts/default.toml"),
            r#"
extensions = ["builtin:frontend-tools", "local:project-tools"]

[pi]
profile = "voice"
model = "LM-Studio/gemma-4-26b-a4b-it"
thinking = "minimal"
append_system_prompt_file = "SYSTEM.md"

[tools]
required_frontend = ["codec.display"]
allowed_pi = ["read", "write"]

[adapters]
stt = "parakeet-silero"
tts = "qwen3-worker"

[lifecycle]
prewarm = true
keep_warm_ms = 600000
"#,
        )
        .unwrap();

        let resolver = LoadoutResolver::new(LoadoutConfig {
            default_name: "default".to_string(),
            bundled_extensions_dir: bundled,
        });

        let resolved = resolver.resolve(dir.path(), None).unwrap();

        assert_eq!(resolved.name, "default");
        assert_eq!(resolved.loadout.pi.profile.as_deref(), Some("voice"));
        assert_eq!(
            resolved.loadout.pi.model.as_deref(),
            Some("LM-Studio/gemma-4-26b-a4b-it")
        );
        assert_eq!(resolved.loadout.pi.thinking.as_deref(), Some("minimal"));
        assert_eq!(
            resolved.loadout.pi.append_system_prompt_file.as_deref(),
            Some("SYSTEM.md")
        );
        assert_eq!(resolved.loadout.tools.required_frontend, ["codec.display"]);
        assert_eq!(resolved.extension_paths, vec![builtin, local]);
        assert!(resolved.loadout.lifecycle.prewarm);
    }

    #[test]
    fn missing_loadout_uses_defaults_without_extensions() {
        let dir = tempdir().unwrap();
        let resolver = LoadoutResolver::new(LoadoutConfig {
            default_name: "default".to_string(),
            bundled_extensions_dir: dir.path().join("bundled"),
        });

        let resolved = resolver.resolve(dir.path(), Some("missing")).unwrap();

        assert_eq!(resolved.name, "missing");
        assert_eq!(resolved.loadout.adapters.stt, "parakeet-silero");
        assert!(resolved.source.is_none());
        assert!(resolved.extension_paths.is_empty());
    }

    #[test]
    fn rejects_missing_extension_reference() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".foxline/loadouts")).unwrap();
        fs::write(
            dir.path().join(".foxline/loadouts/default.toml"),
            r#"extensions = ["builtin:frontend-tools"]"#,
        )
        .unwrap();
        let resolver = LoadoutResolver::new(LoadoutConfig {
            default_name: "default".to_string(),
            bundled_extensions_dir: dir.path().join("bundled"),
        });

        let err = resolver.resolve(dir.path(), None).unwrap_err();

        assert!(err.to_string().contains("missing directory"));
    }
}
