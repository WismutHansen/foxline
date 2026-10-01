//! Engine-specific launch configuration; framing/lifecycle stays in tts.
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{loadout::AdapterLoadout, persona::ResolvedPersona, tts::QwenWorkerConfig};

pub(crate) fn kokorox_worker_config(
    adapters: &AdapterLoadout,
    persona: &ResolvedPersona,
    repo: &Path,
    worker_override: Option<&str>,
) -> Result<QwenWorkerConfig> {
    let voice = persona
        .kokorox_voice
        .as_ref()
        .with_context(|| format!("Persona {:?} has no [voice.kokorox] mapping", persona.id))?;
    let configured = worker_override.or_else(|| {
        adapters
            .kokorox
            .as_ref()
            .map(|config| config.worker.as_str())
    });
    let worker = match configured {
        Some(worker) => explicit_worker(worker)?,
        None => {
            let sibling = repo
                .parent()
                .unwrap_or(repo)
                .join("kokorox/target/release/kokorox-tts-worker");
            if sibling.is_file() {
                sibling
            } else {
                PathBuf::from("kokorox-tts-worker")
            }
        }
    };
    let args = vec![
        "--serve".into(),
        "--model".into(),
        voice.model.to_string_lossy().into_owned(),
        "--voices".into(),
        voice.voices.to_string_lossy().into_owned(),
        "--voice".into(),
        voice.voice.clone(),
        "--language".into(),
        voice.language.clone(),
        "--speed".into(),
        voice.speed.to_string(),
        "--output-sample-rate".into(),
        "24000".into(),
    ];
    let mut config = QwenWorkerConfig::new(worker.to_string_lossy(), args, repo);
    config.backend = "kokorox".into();
    Ok(config)
}

fn explicit_worker(worker: &str) -> Result<PathBuf> {
    let path = PathBuf::from(worker);
    if worker.trim().is_empty() {
        bail!("kokorox worker must not be empty");
    }
    if path.is_absolute() {
        if !path.is_file() {
            bail!("kokorox worker does not exist: {}", path.display());
        }
    } else if path.components().count() != 1 {
        bail!("kokorox worker must be an absolute path or a program name on PATH");
    }
    Ok(path)
}

#[cfg(test)]
#[path = "tts_kokorox_tests.rs"]
mod real_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{loadout::KokoroxWorkerLoadout, persona::PersonaRegistry};
    use std::fs;
    use tempfile::TempDir;

    fn package(temp: &TempDir) -> ResolvedPersona {
        let root = temp.path().join(".foxline/personas/demo");
        fs::create_dir_all(root.join("voice")).unwrap();
        fs::write(root.join("voice/model.onnx"), b"model").unwrap();
        fs::write(root.join("voice/voices.npz"), b"voices").unwrap();
        fs::write(root.join("persona.toml"), "[voice.kokorox]\nmodel='voice/model.onnx'\nvoices='voice/voices.npz'\nvoice='martin'\nlanguage='de'\n").unwrap();
        PersonaRegistry::new(temp.path())
            .resolve("demo", temp.path())
            .unwrap()
    }

    #[test]
    fn launch_uses_only_native_flags_and_explicit_persona() {
        let temp = TempDir::new().unwrap();
        let persona = package(&temp);
        let adapters = AdapterLoadout {
            kokorox: Some(KokoroxWorkerLoadout {
                worker: "configured-worker".into(),
            }),
            ..Default::default()
        };
        let config = kokorox_worker_config(&adapters, &persona, temp.path(), None).unwrap();
        assert_eq!(config.command, "configured-worker");
        assert_eq!(config.backend, "kokorox");
        assert_eq!(config.output_sample_rate_hz, 24000);
        for forbidden in [
            "--ref-audio",
            "--ref-text-file",
            "--temperature",
            "--model-name",
        ] {
            assert!(!config.args.iter().any(|arg| arg == forbidden));
        }
        assert_eq!(
            &config.args[5..11],
            ["--voice", "martin", "--language", "de", "--speed", "1"]
        );
        let overridden =
            kokorox_worker_config(&adapters, &persona, temp.path(), Some("overridden-worker"))
                .unwrap();
        assert_eq!(overridden.command, "overridden-worker");
    }

    #[test]
    fn missing_mapping_does_not_fallback_to_qwen() {
        let temp = TempDir::new().unwrap();
        let mut persona = package(&temp);
        persona.kokorox_voice = None;
        let error = kokorox_worker_config(
            &AdapterLoadout::default(),
            &persona,
            temp.path(),
            Some("worker"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("[voice.kokorox]"));
    }

    #[test]
    fn worker_paths_are_explicit_and_not_silently_substituted() {
        assert!(explicit_worker("").is_err());
        assert!(explicit_worker("./worker").is_err());
        assert!(explicit_worker("/nonexistent/worker").is_err());
        assert_eq!(explicit_worker("worker").unwrap(), PathBuf::from("worker"));
    }
}
