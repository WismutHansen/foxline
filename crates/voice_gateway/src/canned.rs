use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use bytes::Bytes;

use crate::persona::ResolvedPersona;

const OUTPUT_SAMPLE_RATE_HZ: u32 = 24_000;
const CHUNK_BYTES: usize = 4_096;
const TOOL_SLOW_AFTER: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CannedEvent {
    ToolStarted,
    ToolSlow,
    ToolCompleted,
}

impl CannedEvent {
    fn key(self) -> &'static str {
        match self {
            Self::ToolStarted => "tool_started",
            Self::ToolSlow => "tool_slow",
            Self::ToolCompleted => "tool_completed",
        }
    }
}

#[derive(Debug, Default)]
pub struct CannedSpeech {
    groups: BTreeMap<String, Vec<PathBuf>>,
    next_indices: BTreeMap<String, usize>,
    pending: VecDeque<Bytes>,
    tool_started_at: Option<Instant>,
    slow_emitted: bool,
}

impl CannedSpeech {
    pub fn from_persona(persona: &ResolvedPersona) -> Result<Self> {
        Self::from_directories(&persona.canned)
    }

    fn from_directories(directories: &BTreeMap<String, PathBuf>) -> Result<Self> {
        let mut groups = BTreeMap::new();
        for (event, directory) in directories {
            let mut files = fs::read_dir(directory)
                .with_context(|| format!("read canned speech directory {}", directory.display()))?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
                })
                .collect::<Vec<_>>();
            files.sort();
            groups.insert(event.clone(), files);
        }
        Ok(Self {
            groups,
            ..Self::default()
        })
    }

    pub fn tool_started(&mut self, tts_active: bool) -> Result<bool> {
        self.tool_started_at = Some(Instant::now());
        self.slow_emitted = false;
        self.play(CannedEvent::ToolStarted, tts_active)
    }

    pub fn tool_completed(&mut self, tts_active: bool) -> Result<bool> {
        self.tool_started_at = None;
        self.slow_emitted = false;
        self.play(CannedEvent::ToolCompleted, tts_active)
    }

    pub fn poll_tool_slow(&mut self, now: Instant, tts_active: bool) -> Result<bool> {
        let Some(started) = self.tool_started_at else {
            return Ok(false);
        };
        if self.slow_emitted || now.duration_since(started) < TOOL_SLOW_AFTER {
            return Ok(false);
        }
        self.slow_emitted = true;
        self.play(CannedEvent::ToolSlow, tts_active)
    }

    fn play(&mut self, event: CannedEvent, tts_active: bool) -> Result<bool> {
        if tts_active || self.is_playing() {
            return Ok(false);
        }
        let key = event.key();
        let Some(files) = self.groups.get(key) else {
            return Ok(false);
        };
        if files.is_empty() {
            return Ok(false);
        }
        let index = self.next_indices.entry(key.to_string()).or_default();
        let path = &files[*index % files.len()];
        *index = (*index + 1) % files.len();
        let pcm = read_pcm16_wav(path)?;
        self.pending
            .extend(pcm.chunks(CHUNK_BYTES).map(Bytes::copy_from_slice));
        Ok(!self.pending.is_empty())
    }

    pub fn next_chunk(&mut self) -> Option<Bytes> {
        self.pending.pop_front()
    }

    pub fn cancel(&mut self) -> bool {
        let was_playing = self.is_playing();
        self.pending.clear();
        self.tool_started_at = None;
        self.slow_emitted = false;
        was_playing
    }

    pub fn is_playing(&self) -> bool {
        !self.pending.is_empty()
    }

    #[cfg(test)]
    fn set_tool_started_at(&mut self, instant: Instant) {
        self.tool_started_at = Some(instant);
    }
}

fn read_pcm16_wav(path: &Path) -> Result<Vec<u8>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open canned speech WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        bail!(
            "canned speech WAV must use signed PCM16: {}",
            path.display()
        );
    }
    if spec.sample_rate != OUTPUT_SAMPLE_RATE_HZ {
        bail!(
            "canned speech WAV must be {OUTPUT_SAMPLE_RATE_HZ} Hz (found {}): {}",
            spec.sample_rate,
            path.display()
        );
    }
    if spec.channels == 0 || spec.channels > 2 {
        bail!(
            "canned speech WAV must be mono or stereo: {}",
            path.display()
        );
    }
    let samples = reader
        .samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("decode canned speech WAV {}", path.display()))?;
    let mono = if spec.channels == 1 {
        samples
    } else {
        samples
            .chunks_exact(2)
            .map(|pair| {
                let mixed = (i32::from(pair[0]) + i32::from(pair[1])) / 2;
                mixed as i16
            })
            .collect()
    };
    Ok(mono
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_wav(path: &Path, value: i16, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: OUTPUT_SAMPLE_RATE_HZ,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..channels {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn player(temp: &TempDir) -> CannedSpeech {
        let started = temp.path().join("started");
        let slow = temp.path().join("slow");
        let completed = temp.path().join("completed");
        for directory in [&started, &slow, &completed] {
            fs::create_dir_all(directory).unwrap();
        }
        write_wav(&started.join("01.wav"), 1, 1);
        write_wav(&started.join("02.wav"), 2, 1);
        write_wav(&slow.join("01.wav"), 3, 2);
        write_wav(&completed.join("01.wav"), 4, 1);
        CannedSpeech::from_directories(&BTreeMap::from([
            ("tool_started".to_string(), started),
            ("tool_slow".to_string(), slow),
            ("tool_completed".to_string(), completed),
        ]))
        .unwrap()
    }

    #[test]
    fn lifecycle_groups_map_and_started_selection_is_round_robin() {
        let temp = TempDir::new().unwrap();
        let mut speech = player(&temp);
        assert!(speech.tool_started(false).unwrap());
        assert_eq!(speech.next_chunk().unwrap().as_ref(), &1i16.to_le_bytes());
        assert!(speech.tool_completed(false).unwrap());
        assert_eq!(speech.next_chunk().unwrap().as_ref(), &4i16.to_le_bytes());
        assert!(speech.tool_started(false).unwrap());
        assert_eq!(speech.next_chunk().unwrap().as_ref(), &2i16.to_le_bytes());
    }

    #[test]
    fn empty_or_absent_group_is_a_noop() {
        let mut speech = CannedSpeech::default();
        assert!(!speech.tool_started(false).unwrap());
        assert!(speech.next_chunk().is_none());
    }

    #[test]
    fn active_tts_and_existing_canned_playback_prevent_overlap() {
        let temp = TempDir::new().unwrap();
        let mut speech = player(&temp);
        assert!(!speech.tool_started(true).unwrap());
        assert!(speech.tool_started(false).unwrap());
        assert!(!speech.tool_completed(false).unwrap());
    }

    #[test]
    fn interrupt_clears_session_local_playback() {
        let temp = TempDir::new().unwrap();
        let mut first = player(&temp);
        let mut second = player(&temp);
        assert!(first.tool_started(false).unwrap());
        assert!(second.tool_started(false).unwrap());
        assert!(first.cancel());
        assert!(first.next_chunk().is_none());
        assert!(second.next_chunk().is_some());
    }

    #[test]
    fn slow_group_plays_once_after_threshold() {
        let temp = TempDir::new().unwrap();
        let mut speech = player(&temp);
        let now = Instant::now();
        speech.set_tool_started_at(now - TOOL_SLOW_AFTER);
        assert!(speech.poll_tool_slow(now, false).unwrap());
        assert_eq!(speech.next_chunk().unwrap().as_ref(), &3i16.to_le_bytes());
        assert!(!speech.poll_tool_slow(now + TOOL_SLOW_AFTER, false).unwrap());
    }
}
