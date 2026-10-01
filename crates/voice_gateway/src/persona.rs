use std::{
    collections::BTreeMap,
    env, fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceReference {
    pub wav: PathBuf,
    pub txt: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KokoroxVoice {
    pub model: PathBuf,
    pub voices: PathBuf,
    pub voice: String,
    pub language: String,
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPersona {
    pub id: String,
    pub root: PathBuf,
    pub prompt: Option<String>,
    pub digest: String,
    /// Explicit spqx mapping, or compatible legacy reference fields.
    pub voice: Option<VoiceReference>,
    pub kokorox_voice: Option<KokoroxVoice>,
    pub canned: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PersonaRegistry {
    bundled_root: PathBuf,
    data_root: PathBuf,
    legacy_agents_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
struct PersonaManifest {
    #[serde(default)]
    voice: Option<VoiceManifest>,
    #[serde(default)]
    canned: BTreeMap<String, CannedGroup>,
}

#[derive(Debug, Deserialize, Default)]
struct VoiceManifest {
    reference_audio: Option<String>,
    reference_text: Option<String>,
    spqx: Option<ReferenceManifest>,
    kokorox: Option<KokoroxManifest>,
    // Retain existing descriptive metadata and voice.variants arrays. Unknown
    // table-valued sections are rejected instead of hiding misspelled engines.
    #[serde(flatten)]
    metadata: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceManifest {
    reference_audio: String,
    reference_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KokoroxManifest {
    model: String,
    voices: String,
    voice: String,
    language: String,
    #[serde(default = "default_speed")]
    speed: f32,
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Debug, Deserialize)]
struct CannedGroup {
    directory: String,
}

impl PersonaRegistry {
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self::new_with_data_root(repo_root, default_data_root())
    }

    pub(crate) fn new_with_data_root(
        repo_root: impl AsRef<Path>,
        data_root: impl Into<PathBuf>,
    ) -> Self {
        let repo_root = repo_root.as_ref();
        Self {
            bundled_root: repo_root.join("personas"),
            data_root: data_root.into(),
            legacy_agents_root: Some(repo_root.join("agents")),
        }
    }

    #[cfg(test)]
    fn with_roots(data_root: PathBuf, bundled_root: PathBuf) -> Self {
        Self {
            data_root,
            bundled_root,
            legacy_agents_root: None,
        }
    }

    pub fn resolve(&self, id: &str, workspace: &Path) -> Result<ResolvedPersona> {
        validate_id(id)?;
        let mut roots = vec![
            workspace.join(".foxline/personas").join(id),
            self.data_root.join(id),
            self.bundled_root.join(id),
            // Compatibility with pre-registry sidecar layouts.
            workspace.join("personas").join(id),
            workspace.join("agents").join(id),
        ];
        if let Some(legacy) = &self.legacy_agents_root {
            roots.push(legacy.join(id));
        }
        let root = roots
            .into_iter()
            .find(|candidate| candidate.is_dir())
            .with_context(|| format!("Persona {id:?} was not found in the Work Directory, XDG data directory, or bundled Personas"))?;
        resolve_package(id, root)
    }
}

fn default_data_root() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("foxline/personas")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/foxline/personas")
    } else {
        PathBuf::from(".local/share/foxline/personas")
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("invalid Persona id {id:?}");
    }
    Ok(())
}

fn safe_relative(root: &Path, value: &str, field: &str) -> Result<PathBuf> {
    let relative = Path::new(value);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("Persona {field} must be a confined relative path: {value:?}");
    }
    let joined = root.join(relative);
    if joined.exists() {
        let canonical_root = root
            .canonicalize()
            .with_context(|| format!("canonicalize Persona root {}", root.display()))?;
        let canonical_joined = joined
            .canonicalize()
            .with_context(|| format!("canonicalize Persona path {}", joined.display()))?;
        if !canonical_joined.starts_with(&canonical_root) {
            bail!("Persona {field} escapes the Persona package: {value:?}");
        }
        return Ok(joined);
    }
    Ok(joined)
}

fn resolve_package(id: &str, root: PathBuf) -> Result<ResolvedPersona> {
    // Worker cwd is independent of the selected Work Directory.
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .context("resolve Persona package root")?
            .join(root)
    };
    let manifest_path = root.join("persona.toml");
    let manifest_bytes = if manifest_path.exists() {
        fs::read(&manifest_path).with_context(|| format!("read {}", manifest_path.display()))?
    } else {
        Vec::new()
    };
    let manifest: PersonaManifest = if manifest_bytes.is_empty() {
        PersonaManifest::default()
    } else {
        toml::from_str(
            std::str::from_utf8(&manifest_bytes).context("Persona manifest is not UTF-8")?,
        )
        .with_context(|| format!("parse {}", manifest_path.display()))?
    };

    let prompt_path = root.join("PROMPT.md");
    let prompt_bytes = if prompt_path.exists() {
        fs::read(&prompt_path).with_context(|| format!("read {}", prompt_path.display()))?
    } else {
        Vec::new()
    };
    let prompt = if prompt_bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8(prompt_bytes.clone()).context("PROMPT.md is not UTF-8")?)
    };

    let (voice, kokorox_voice) = resolve_voices(id, &root, manifest.voice.unwrap_or_default())?;

    let mut canned = BTreeMap::new();
    for (event, group) in manifest.canned {
        let path = safe_relative(
            &root,
            &group.directory,
            &format!("canned.{event}.directory"),
        )?;
        if !path.is_dir() {
            bail!(
                "Persona {id:?} canned directory does not exist: {}",
                path.display()
            );
        }
        canned.insert(event, path);
    }

    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update(&manifest_bytes);
    hasher.update(&prompt_bytes);
    let digest = format!("{:x}", hasher.finalize());
    Ok(ResolvedPersona {
        id: id.to_string(),
        root,
        prompt,
        digest,
        voice,
        kokorox_voice,
        canned,
    })
}

fn resolve_voices(
    id: &str,
    root: &Path,
    voice: VoiceManifest,
) -> Result<(Option<VoiceReference>, Option<KokoroxVoice>)> {
    for (section, value) in &voice.metadata {
        if value.is_table() {
            bail!("Persona {id:?} has unknown voice section {section:?}");
        }
    }
    let explicit = voice.spqx.is_some() || voice.kokorox.is_some();
    let legacy = match (voice.reference_audio, voice.reference_text) {
        (Some(wav), Some(txt)) => Some(resolve_reference(root, &wav, &txt, "voice")?),
        (None, None) => None,
        _ => bail!(
            "Persona {id:?} voice reference_audio and reference_text must be configured together"
        ),
    };
    let reference = match voice.spqx {
        Some(mapping) => Some(resolve_reference(
            root,
            &mapping.reference_audio,
            &mapping.reference_text,
            "voice.spqx",
        )?),
        None => legacy.or_else(|| {
            if explicit {
                None
            } else {
                legacy_voice_reference(root)
            }
        }),
    };
    let kokorox = voice
        .kokorox
        .map(|mapping| resolve_kokorox_voice(root, mapping))
        .transpose()?;
    Ok((reference, kokorox))
}

fn resolve_reference(root: &Path, wav: &str, txt: &str, section: &str) -> Result<VoiceReference> {
    Ok(VoiceReference {
        wav: existing_asset(root, wav, &format!("{section}.reference_audio"))?,
        txt: existing_asset(root, txt, &format!("{section}.reference_text"))?,
    })
}

fn existing_asset(root: &Path, value: &str, field: &str) -> Result<PathBuf> {
    let path = safe_relative(root, value, field)?;
    if !path.is_file() {
        bail!("Persona {field} requires an existing file: {value:?}");
    }
    Ok(path)
}

fn resolve_kokorox_voice(root: &Path, mapping: KokoroxManifest) -> Result<KokoroxVoice> {
    if mapping.voice.trim().is_empty()
        || mapping.voice.chars().any(char::is_whitespace)
        || mapping.voice.contains('+')
    {
        bail!("voice.kokorox.voice must be a single explicit voice id (mixing unsupported)");
    }
    if mapping.language.trim().is_empty()
        || mapping.language == "auto"
        || mapping.language.starts_with("zh")
    {
        bail!("voice.kokorox.language must explicitly select a v1.0-vocabulary language; auto/Chinese v1.1 unsupported");
    }
    if !mapping.speed.is_finite() || mapping.speed <= 0.0 {
        bail!("voice.kokorox.speed must be finite and positive");
    }
    Ok(KokoroxVoice {
        model: existing_asset(root, &mapping.model, "voice.kokorox.model")?,
        voices: existing_asset(root, &mapping.voices, "voice.kokorox.voices")?,
        voice: mapping.voice,
        language: mapping.language,
        speed: mapping.speed,
    })
}

fn legacy_voice_reference(root: &Path) -> Option<VoiceReference> {
    for dir in [
        root.join("voice/reference_audio"),
        root.join("assets/reference_audio"),
        root.join("assets"),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let wav = entry.path();
            if !wav
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
            {
                continue;
            }
            let text = [
                wav.with_extension("txt"),
                dir.join("reference.txt"),
                PathBuf::from(format!("{}.txt", wav.display())),
            ]
            .into_iter()
            .find(|candidate| candidate.is_file());
            if let Some(text) = text {
                // Legacy discovery is not a path-confinement bypass.
                let wav = wav.strip_prefix(root).ok()?.to_str()?;
                let txt = text.strip_prefix(root).ok()?.to_str()?;
                return resolve_reference(root, wav, txt, "voice").ok();
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "persona_voice_tests.rs"]
mod voice_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn package(root: &Path, id: &str, prompt: &str) {
        let dir = root.join(id);
        fs::create_dir_all(dir.join("voice")).unwrap();
        fs::write(dir.join("PROMPT.md"), prompt).unwrap();
        fs::write(dir.join("voice/ref.wav"), b"wav").unwrap();
        fs::write(dir.join("voice/ref.txt"), "words").unwrap();
        fs::write(
            dir.join("persona.toml"),
            "[voice]\nreference_audio = \"voice/ref.wav\"\nreference_text = \"voice/ref.txt\"\n",
        )
        .unwrap();
    }

    #[test]
    fn workspace_persona_overrides_xdg_and_bundled() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("work");
        let data = temp.path().join("data");
        let bundled = temp.path().join("bundled");
        package(&workspace.join(".foxline/personas"), "kitt", "workspace");
        package(&data, "kitt", "data");
        package(&bundled, "kitt", "bundled");
        let persona = PersonaRegistry::with_roots(data, bundled)
            .resolve("kitt", &workspace)
            .unwrap();
        assert_eq!(persona.prompt.as_deref(), Some("workspace"));
    }

    #[test]
    fn xdg_persona_overrides_bundled() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("data");
        let bundled = temp.path().join("bundled");
        package(&data, "kitt", "data");
        package(&bundled, "kitt", "bundled");
        let persona = PersonaRegistry::with_roots(data, bundled)
            .resolve("kitt", &temp.path().join("work"))
            .unwrap();
        assert_eq!(persona.prompt.as_deref(), Some("data"));
    }

    #[test]
    fn prompt_changes_persona_digest() {
        let temp = TempDir::new().unwrap();
        package(temp.path(), "kitt", "first");
        let registry =
            PersonaRegistry::with_roots(temp.path().to_path_buf(), temp.path().join("bundled"));
        let first = registry.resolve("kitt", temp.path()).unwrap();
        fs::write(temp.path().join("kitt/PROMPT.md"), "second").unwrap();
        let second = registry.resolve("kitt", temp.path()).unwrap();
        assert_ne!(first.digest, second.digest);
    }

    #[test]
    fn rejects_traversal_and_incomplete_voice_pairs() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("kitt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("persona.toml"),
            "[voice]\nreference_audio = \"../ref.wav\"\n",
        )
        .unwrap();
        let registry =
            PersonaRegistry::with_roots(temp.path().to_path_buf(), temp.path().join("bundled"));
        assert!(registry.resolve("../kitt", temp.path()).is_err());
        assert!(registry
            .resolve("kitt", temp.path())
            .unwrap_err()
            .to_string()
            .contains("configured together"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_from_persona_package() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("kitt");
        fs::create_dir_all(dir.join("voice")).unwrap();
        fs::write(temp.path().join("outside.wav"), b"wav").unwrap();
        fs::write(dir.join("voice/ref.txt"), "words").unwrap();
        symlink(temp.path().join("outside.wav"), dir.join("voice/ref.wav")).unwrap();
        fs::write(
            dir.join("persona.toml"),
            "[voice]\nreference_audio = \"voice/ref.wav\"\nreference_text = \"voice/ref.txt\"\n",
        )
        .unwrap();
        let registry =
            PersonaRegistry::with_roots(temp.path().to_path_buf(), temp.path().join("bundled"));
        assert!(registry
            .resolve("kitt", temp.path())
            .unwrap_err()
            .to_string()
            .contains("escapes"));
    }
}
