use super::*;
use tempfile::TempDir;

fn package(temp: &TempDir, manifest: &str) -> Result<ResolvedPersona> {
    let root = temp.path().join(".foxline/personas/demo");
    fs::create_dir_all(root.join("voice/reference_audio"))?;
    for file in ["model.onnx", "voices.npz", "explicit.wav", "explicit.txt"] {
        fs::write(root.join("voice").join(file), b"fixture")?;
    }
    fs::write(root.join("voice/reference_audio/legacy.wav"), b"fixture")?;
    fs::write(root.join("voice/reference_audio/reference.txt"), b"legacy")?;
    fs::write(root.join("persona.toml"), manifest)?;
    PersonaRegistry::new_with_data_root(temp.path(), temp.path().join("data"))
        .resolve("demo", temp.path())
}

const KOKOROX: &str = "[voice.kokorox]\nmodel='voice/model.onnx'\nvoices='voice/voices.npz'\nvoice='martin'\nlanguage='de'\n";
const SPQX: &str =
    "[voice.spqx]\nreference_audio='voice/explicit.wav'\nreference_text='voice/explicit.txt'\n";

#[test]
fn relative_package_roots_produce_absolute_worker_assets() {
    let cwd = std::env::current_dir().unwrap();
    let temp = TempDir::new_in(&cwd).unwrap();
    let persona = package(&temp, KOKOROX).unwrap();
    let relative = persona.root.strip_prefix(&cwd).unwrap().to_path_buf();
    let resolved = resolve_package("demo", relative).unwrap();
    assert!(resolved.root.is_absolute());
    assert!(resolved.kokorox_voice.unwrap().model.is_absolute());
}

#[test]
fn explicit_engine_mappings_are_independent() {
    let temp = TempDir::new().unwrap();
    let persona = package(&temp, &format!("{KOKOROX}\n{SPQX}")).unwrap();
    assert!(persona.voice.unwrap().wav.ends_with("voice/explicit.wav"));
    let voice = persona.kokorox_voice.unwrap();
    assert_eq!(voice.voice, "martin");
    assert_eq!(voice.language, "de");
    assert_eq!(voice.speed, 1.0);
}

#[test]
fn kokorox_only_mapping_does_not_pick_legacy_directory_for_spqx() {
    let temp = TempDir::new().unwrap();
    let persona = package(&temp, KOKOROX).unwrap();
    assert!(persona.voice.is_none());
    assert!(persona.kokorox_voice.is_some());
    let persona = package(&temp, SPQX).unwrap();
    assert!(persona.kokorox_voice.is_none());
}

#[test]
fn legacy_fields_and_descriptive_metadata_remain_compatible() {
    let temp = TempDir::new().unwrap();
    let legacy = "[voice]\nadapter='qwen3-worker'\ndescription='fixture'\nreference_audio='voice/explicit.wav'\nreference_text='voice/explicit.txt'\n[[voice.variants]]\nid='old'\n";
    let persona = package(&temp, &format!("{legacy}\n{KOKOROX}")).unwrap();
    assert!(persona.voice.unwrap().wav.ends_with("voice/explicit.wav"));
    assert!(persona.kokorox_voice.is_some());
    assert!(package(&temp, "")
        .unwrap()
        .voice
        .unwrap()
        .wav
        .ends_with("legacy.wav"));
}

#[test]
fn explicit_spqx_takes_precedence_over_legacy_fields() {
    let temp = TempDir::new().unwrap();
    let legacy = "[voice]\nreference_audio='voice/reference_audio/legacy.wav'\nreference_text='voice/reference_audio/reference.txt'\n";
    assert!(package(&temp, &format!("{legacy}{SPQX}"))
        .unwrap()
        .voice
        .unwrap()
        .wav
        .ends_with("explicit.wav"));
}

#[test]
fn misspelled_or_unsupported_engine_fields_fail_instead_of_falling_back() {
    let temp = TempDir::new().unwrap();
    for invalid in [
        SPQX.replace("spqx", "spxq"),
        format!("{SPQX}speed=1.2\n"),
        format!("{KOKOROX}reference_audio='voice/explicit.wav'\n"),
        KOKOROX.replace("language='de'\n", ""),
    ] {
        assert!(package(&temp, &invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn invalid_kokorox_options_are_rejected() {
    let temp = TempDir::new().unwrap();
    for invalid in [
        format!("{KOKOROX}speed=0\n"),
        format!("{KOKOROX}speed=-1\n"),
        format!("{KOKOROX}speed=nan\n"),
        format!("{KOKOROX}speed=inf\n"),
        KOKOROX.replace("'martin'", "'af_heart+af_bella'"),
        KOKOROX.replace("'de'", "'auto'"),
        KOKOROX.replace("'de'", "'zh'"),
    ] {
        assert!(package(&temp, &invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn engine_asset_paths_are_confined_existing_files() {
    let temp = TempDir::new().unwrap();
    for (manifest, from) in [
        (KOKOROX, "voice/model.onnx"),
        (KOKOROX, "voice/voices.npz"),
        (SPQX, "voice/explicit.wav"),
        (SPQX, "voice/explicit.txt"),
    ] {
        for to in ["../outside", "/tmp/outside", "voice/missing", "voice"] {
            assert!(
                package(&temp, &manifest.replace(from, to)).is_err(),
                "accepted {to}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn engine_assets_cannot_escape_through_symlinks() {
    let temp = TempDir::new().unwrap();
    package(&temp, KOKOROX).unwrap();
    let outside = temp.path().join("outside.onnx");
    fs::write(&outside, b"fixture").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        temp.path().join(".foxline/personas/demo/voice/link.onnx"),
    )
    .unwrap();
    assert!(package(
        &temp,
        &KOKOROX.replace("voice/model.onnx", "voice/link.onnx")
    )
    .is_err());
}
