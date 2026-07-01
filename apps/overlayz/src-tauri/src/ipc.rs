use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    ToggleAudio,
    ShowKitt,
    ShowOrb,
    ShowLastAgent,
    Hide,
    GetStatus,
    SetTheme { name: String },
    ListMicrophones,
    SetMicrophone { device_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrophoneDevice {
    pub device_id: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: String,
    pub status: Option<AppStatus>,
    pub microphones: Option<Vec<MicrophoneDevice>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub visible: bool,
    pub agent: String,
    pub audio_enabled: bool,
    pub state: String,
}

pub fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("overlayz.sock")
    } else {
        std::env::temp_dir().join("overlayz.sock")
    }
}
