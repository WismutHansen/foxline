use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationHistory {
    pub messages: Vec<Message>,
}

impl ConversationHistory {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn add_message(&mut self, role: String, content: String) {
        self.messages.push(Message { role, content });
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn save(
        &self,
        history_dir: &PathBuf,
        ui_mode: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(history_dir)?;
        let file_path = history_dir.join(format!("{}.json", ui_mode));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(file_path, json)?;
        Ok(())
    }

    pub fn load(ui_mode: &str, history_dir: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let file_path = history_dir.join(format!("{}.json", ui_mode));
        if !file_path.exists() {
            return Ok(Self::new());
        }
        let json = fs::read_to_string(file_path)?;
        let history = serde_json::from_str(&json)?;
        Ok(history)
    }
}
