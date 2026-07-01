use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager, PhysicalPosition, State};

mod config;
mod history;
pub mod ipc;
mod theme;

use config::AppConfig;
use history::ConversationHistory;
use ipc::{AppStatus, IpcCommand, IpcResponse};
use theme::{OrbColors, ThemeManager};

use ipc::MicrophoneDevice as IpcMicrophoneDevice;

struct AppState {
    config: Mutex<AppConfig>,
    kitt_history: Mutex<ConversationHistory>,
    orb_history: Mutex<ConversationHistory>,
    audio_enabled: Mutex<bool>,
    last_agent: Mutex<String>,
    selected_microphone: Mutex<Option<String>>,
    cached_microphones: Mutex<Vec<IpcMicrophoneDevice>>,
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    state
        .config
        .lock()
        .map(|config| config.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_history(ui_mode: String, state: State<AppState>) -> Result<ConversationHistory, String> {
    match ui_mode.as_str() {
        "kitt" => state
            .kitt_history
            .lock()
            .map(|h| h.clone())
            .map_err(|e| e.to_string()),
        "orb" => state
            .orb_history
            .lock()
            .map(|h| h.clone())
            .map_err(|e| e.to_string()),
        _ => Err("Invalid UI mode".to_string()),
    }
}

#[tauri::command]
fn add_message(
    ui_mode: String,
    role: String,
    content: String,
    state: State<AppState>,
) -> Result<(), String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let history_dir = config
        .storage
        .history_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| AppConfig::data_dir().join("history"));

    let ui_mode_str = ui_mode.as_str();
    match ui_mode_str {
        "kitt" => {
            let mut history = state.kitt_history.lock().map_err(|e| e.to_string())?;
            history.add_message(role, content);
            history
                .save(&history_dir, ui_mode_str)
                .map_err(|e| e.to_string())?;
        }
        "orb" => {
            let mut history = state.orb_history.lock().map_err(|e| e.to_string())?;
            history.add_message(role, content);
            history
                .save(&history_dir, ui_mode_str)
                .map_err(|e| e.to_string())?;
        }
        _ => return Err("Invalid UI mode".to_string()),
    }
    Ok(())
}

#[tauri::command]
fn clear_history(ui_mode: String, state: State<AppState>) -> Result<(), String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let history_dir = config
        .storage
        .history_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| AppConfig::data_dir().join("history"));

    let ui_mode_str = ui_mode.as_str();
    match ui_mode_str {
        "kitt" => {
            let mut history = state.kitt_history.lock().map_err(|e| e.to_string())?;
            history.clear();
            history
                .save(&history_dir, ui_mode_str)
                .map_err(|e| e.to_string())?;
        }
        "orb" => {
            let mut history = state.orb_history.lock().map_err(|e| e.to_string())?;
            history.clear();
            history
                .save(&history_dir, ui_mode_str)
                .map_err(|e| e.to_string())?;
        }
        _ => return Err("Invalid UI mode".to_string()),
    }
    Ok(())
}

#[tauri::command]
fn set_last_agent(agent: String, state: State<AppState>) -> Result<(), String> {
    let mut last_agent = state.last_agent.lock().map_err(|e| e.to_string())?;
    *last_agent = agent;
    Ok(())
}

#[tauri::command]
fn get_audio_enabled(state: State<AppState>) -> Result<bool, String> {
    state
        .audio_enabled
        .lock()
        .map(|enabled| *enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_audio_enabled(enabled: bool, state: State<AppState>) -> Result<(), String> {
    let mut audio_enabled = state.audio_enabled.lock().map_err(|e| e.to_string())?;
    *audio_enabled = enabled;
    Ok(())
}

#[tauri::command]
fn get_orb_colors(state: State<AppState>) -> Result<OrbColors, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    if let Some(theme_config) = &config.orb.theme {
        ThemeManager::get_orb_colors(&theme_config.provider).map_err(|e| e.to_string())
    } else {
        Err("No theme configuration found for orb".to_string())
    }
}

#[tauri::command]
fn get_current_theme() -> Result<String, String> {
    ThemeManager::get_current_theme().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_theme(theme_name: String) -> Result<(), String> {
    ThemeManager::set_current_theme(&theme_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_selected_microphone(state: State<AppState>) -> Result<Option<String>, String> {
    state
        .selected_microphone
        .lock()
        .map(|mic| mic.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_selected_microphone(device_id: String, state: State<AppState>) -> Result<(), String> {
    let mut selected = state
        .selected_microphone
        .lock()
        .map_err(|e| e.to_string())?;
    *selected = Some(device_id);
    Ok(())
}

#[tauri::command]
fn update_microphone_list(
    microphones: Vec<IpcMicrophoneDevice>,
    state: State<AppState>,
) -> Result<(), String> {
    let mut cached = state.cached_microphones.lock().map_err(|e| e.to_string())?;
    *cached = microphones;
    Ok(())
}

#[tauri::command]
fn get_microphone_list(state: State<AppState>) -> Result<Vec<IpcMicrophoneDevice>, String> {
    let cached = state.cached_microphones.lock().map_err(|e| e.to_string())?;
    Ok(cached.clone())
}

fn toggle_window_visibility(app_handle: &tauri::AppHandle, target_agent: Option<&str>) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let is_visible = window.is_visible().unwrap_or(false);

        if let Some(agent) = target_agent {
            // Specific agent requested (kitt or orb)
            let state = app_handle.state::<AppState>();
            let last_agent = state.last_agent.lock().unwrap_or_else(|e| e.into_inner());

            if !is_visible || *last_agent != agent {
                // Show and switch to target agent
                let _ = window.show();
                let _ = window.set_focus();
                let event_name = format!("show_{}_shortcut", agent);
                let _ = app_handle.emit(&event_name, ());
            } else {
                // Already showing target agent, so hide
                let _ = window.hide();
            }
        } else {
            // No specific agent, just toggle visibility
            if !is_visible {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = app_handle.emit("show_last_agent_shortcut", ());
            } else {
                let _ = window.hide();
            }
        }
    }
}

fn start_ipc_server(app_handle: tauri::AppHandle) {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let socket_path = ipc::socket_path();

    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    thread::spawn(move || {
        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => {
                println!(
                    "[IPC] Socket server listening at: {}",
                    socket_path.display()
                );
                l
            }
            Err(e) => {
                eprintln!("[IPC] Failed to bind socket: {}", e);
                return;
            }
        };

        for stream in listener.incoming() {
            let app_handle = app_handle.clone();
            match stream {
                Ok(stream) => {
                    thread::spawn(move || {
                        handle_ipc_connection(stream, app_handle);
                    });
                }
                Err(e) => {
                    eprintln!("[IPC] Connection failed: {}", e);
                }
            }
        }
    });
}

fn handle_ipc_connection(mut stream: std::os::unix::net::UnixStream, app_handle: tauri::AppHandle) {
    use std::io::{BufRead, BufReader, Write};

    let stream_clone = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[IPC] Failed to clone stream: {}", e);
            return;
        }
    };

    let mut reader = BufReader::new(stream_clone);
    let mut line = String::new();

    if let Err(e) = reader.read_line(&mut line) {
        eprintln!("[IPC] Failed to read command: {}", e);
        return;
    }

    let command: IpcCommand = match serde_json::from_str(&line) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("[IPC] Failed to parse command: {}", e);
            let response = IpcResponse {
                success: false,
                message: format!("Invalid command format: {}", e),
                status: None,
                microphones: None,
            };
            let _ = stream.write_all(serde_json::to_string(&response).unwrap().as_bytes());
            return;
        }
    };

    println!("[IPC] Received command: {:?}", command);

    let response = match command {
        IpcCommand::ToggleAudio => {
            let _ = app_handle.emit("toggle_audio_shortcut", ());
            IpcResponse {
                success: true,
                message: "Audio toggled".to_string(),
                status: None,
                microphones: None,
            }
        }
        IpcCommand::ShowKitt => {
            toggle_window_visibility(&app_handle, Some("kitt"));
            IpcResponse {
                success: true,
                message: "Toggled K.I.T.T. visibility".to_string(),
                status: None,
                microphones: None,
            }
        }
        IpcCommand::ShowOrb => {
            toggle_window_visibility(&app_handle, Some("orb"));
            IpcResponse {
                success: true,
                message: "Toggled Orb visibility".to_string(),
                status: None,
                microphones: None,
            }
        }
        IpcCommand::ShowLastAgent => {
            toggle_window_visibility(&app_handle, None);
            IpcResponse {
                success: true,
                message: "Toggled last agent visibility".to_string(),
                status: None,
                microphones: None,
            }
        }
        IpcCommand::Hide => {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.hide();
                IpcResponse {
                    success: true,
                    message: "Window hidden".to_string(),
                    status: None,
                    microphones: None,
                }
            } else {
                IpcResponse {
                    success: false,
                    message: "Window not found".to_string(),
                    status: None,
                    microphones: None,
                }
            }
        }
        IpcCommand::GetStatus => {
            let state = app_handle.state::<AppState>();
            let audio_enabled = state
                .audio_enabled
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let last_agent = state.last_agent.lock().unwrap_or_else(|e| e.into_inner());

            let visible = if let Some(window) = app_handle.get_webview_window("main") {
                window.is_visible().unwrap_or(false)
            } else {
                false
            };

            let status = AppStatus {
                visible,
                agent: last_agent.clone(),
                audio_enabled: *audio_enabled,
                state: "running".to_string(),
            };

            IpcResponse {
                success: true,
                message: "Status retrieved".to_string(),
                status: Some(status),
                microphones: None,
            }
        }
        IpcCommand::SetTheme { name } => match ThemeManager::set_current_theme(&name) {
            Ok(_) => {
                let _ = app_handle.emit("theme_changed", &name);
                IpcResponse {
                    success: true,
                    message: format!("Theme set to '{}'", name),
                    status: None,
                    microphones: None,
                }
            }
            Err(e) => IpcResponse {
                success: false,
                message: format!("Failed to set theme: {}", e),
                status: None,
                microphones: None,
            },
        },
        IpcCommand::ListMicrophones => {
            let state = app_handle.state::<AppState>();
            let cached_mics = state
                .cached_microphones
                .lock()
                .map(|mics| mics.clone())
                .ok();

            if let Some(mics) = cached_mics {
                if mics.is_empty() {
                    let _ = app_handle.emit("list_microphones_request", ());
                    IpcResponse {
                        success: true,
                        message: "Requesting microphone list from frontend...".to_string(),
                        status: None,
                        microphones: None,
                    }
                } else {
                    IpcResponse {
                        success: true,
                        message: format!("Found {} microphone(s)", mics.len()),
                        status: None,
                        microphones: Some(mics),
                    }
                }
            } else {
                IpcResponse {
                    success: false,
                    message: "Failed to access microphone cache".to_string(),
                    status: None,
                    microphones: None,
                }
            }
        }
        IpcCommand::SetMicrophone { device_id } => {
            let state = app_handle.state::<AppState>();
            let result = state.selected_microphone.lock();

            match result {
                Ok(mut selected) => {
                    *selected = Some(device_id.clone());
                    drop(selected);
                    let _ = app_handle.emit("microphone_changed", &device_id);
                    IpcResponse {
                        success: true,
                        message: format!("Microphone set to device: {}", device_id),
                        status: None,
                        microphones: None,
                    }
                }
                Err(e) => IpcResponse {
                    success: false,
                    message: format!("Failed to set microphone: {}", e),
                    status: None,
                    microphones: None,
                },
            }
        }
    };

    let response_json = serde_json::to_string(&response).unwrap();
    let _ = stream.write_all(response_json.as_bytes());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        AppConfig::default()
    });

    let history_dir = config
        .storage
        .history_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| AppConfig::data_dir().join("history"));

    let kitt_history = ConversationHistory::load("kitt", &history_dir)
        .unwrap_or_else(|_| ConversationHistory::new());

    let orb_history = ConversationHistory::load("orb", &history_dir)
        .unwrap_or_else(|_| ConversationHistory::new());

    let app_state = AppState {
        config: Mutex::new(config.clone()),
        kitt_history: Mutex::new(kitt_history),
        orb_history: Mutex::new(orb_history),
        audio_enabled: Mutex::new(true),
        last_agent: Mutex::new("orb".to_string()),
        selected_microphone: Mutex::new(None),
        cached_microphones: Mutex::new(Vec::new()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(app_state)
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(monitor) = window.current_monitor() {
                    if let Some(monitor) = monitor {
                        if let Ok(window_size) = window.outer_size() {
                            let monitor_size = monitor.size();
                            let x = (monitor_size.width as i32 - window_size.width as i32) / 2;
                            let y = -25;
                            let _ = window.set_position(PhysicalPosition::new(x, y));
                        }
                    }
                }
            }

            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

            let handle = app.handle().clone();
            let toggle_audio_shortcut = config.global_shortcuts.toggle_audio.to_accelerator();
            let show_kitt_shortcut = config.global_shortcuts.show_kitt.to_accelerator();
            let show_orb_shortcut = config.global_shortcuts.show_orb.to_accelerator();
            let show_last_agent_shortcut = config.global_shortcuts.show_last_agent.to_accelerator();

            println!("[Global Shortcuts] Registering shortcuts...");

            let mut registered_count = 0;

            // Register toggle_audio shortcut
            if let Ok(shortcut) = toggle_audio_shortcut.parse::<Shortcut>() {
                let register_result = app.global_shortcut().register(shortcut.clone());
                let handle_clone = handle.clone();
                let handler_result =
                    app.global_shortcut()
                        .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                            let _ = handle_clone.emit("toggle_audio_shortcut", ());
                        });
                match (register_result, handler_result) {
                    (Ok(_), Ok(_)) => {
                        println!(
                            "[Global Shortcuts] ✓ Registered toggle_audio: {} (system-wide)",
                            toggle_audio_shortcut
                        );
                        registered_count += 1;
                    }
                    (Err(e1), _) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to register toggle_audio ({}): {}",
                            toggle_audio_shortcut, e1
                        );
                    }
                    (_, Err(e2)) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to set handler for toggle_audio: {}",
                            e2
                        );
                    }
                }
            }

            // Register show_kitt shortcut
            if let Ok(shortcut) = show_kitt_shortcut.parse::<Shortcut>() {
                let register_result = app.global_shortcut().register(shortcut.clone());
                let handle_clone = handle.clone();
                let handler_result =
                    app.global_shortcut()
                        .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                            toggle_window_visibility(&handle_clone, Some("kitt"));
                        });
                match (register_result, handler_result) {
                    (Ok(_), Ok(_)) => {
                        println!(
                            "[Global Shortcuts] ✓ Registered show_kitt: {} (system-wide)",
                            show_kitt_shortcut
                        );
                        registered_count += 1;
                    }
                    (Err(e1), _) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to register show_kitt ({}): {}",
                            show_kitt_shortcut, e1
                        );
                    }
                    (_, Err(e2)) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to set handler for show_kitt: {}",
                            e2
                        );
                    }
                }
            }

            // Register show_orb shortcut
            if let Ok(shortcut) = show_orb_shortcut.parse::<Shortcut>() {
                let register_result = app.global_shortcut().register(shortcut.clone());
                let handle_clone = handle.clone();
                let handler_result =
                    app.global_shortcut()
                        .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                            toggle_window_visibility(&handle_clone, Some("orb"));
                        });
                match (register_result, handler_result) {
                    (Ok(_), Ok(_)) => {
                        println!(
                            "[Global Shortcuts] ✓ Registered show_orb: {} (system-wide)",
                            show_orb_shortcut
                        );
                        registered_count += 1;
                    }
                    (Err(e1), _) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to register show_orb ({}): {}",
                            show_orb_shortcut, e1
                        );
                    }
                    (_, Err(e2)) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to set handler for show_orb: {}",
                            e2
                        );
                    }
                }
            }

            // Register show_last_agent shortcut
            if let Ok(shortcut) = show_last_agent_shortcut.parse::<Shortcut>() {
                let register_result = app.global_shortcut().register(shortcut.clone());
                let handle_clone = handle.clone();
                let handler_result =
                    app.global_shortcut()
                        .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                            toggle_window_visibility(&handle_clone, None);
                        });
                match (register_result, handler_result) {
                    (Ok(_), Ok(_)) => {
                        println!(
                            "[Global Shortcuts] ✓ Registered show_last_agent: {} (system-wide)",
                            show_last_agent_shortcut
                        );
                        registered_count += 1;
                    }
                    (Err(e1), _) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to register show_last_agent ({}): {}",
                            show_last_agent_shortcut, e1
                        );
                    }
                    (_, Err(e2)) => {
                        eprintln!(
                            "[Global Shortcuts] ✗ Failed to set handler for show_last_agent: {}",
                            e2
                        );
                    }
                }
            }

            if registered_count == 0 {
                eprintln!("[Global Shortcuts] Warning: No global shortcuts were registered.");
                eprintln!("[Global Shortcuts] Use 'kittctl' CLI commands for keyboard control.");
            } else {
                println!(
                    "[Global Shortcuts] Successfully registered {}/4 shortcuts.",
                    registered_count
                );
            }

            start_ipc_server(handle.clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_history,
            add_message,
            clear_history,
            set_last_agent,
            get_audio_enabled,
            set_audio_enabled,
            get_orb_colors,
            get_current_theme,
            set_theme,
            get_selected_microphone,
            set_selected_microphone,
            update_microphone_list,
            get_microphone_list
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
