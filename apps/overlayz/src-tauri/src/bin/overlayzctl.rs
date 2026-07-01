use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use overlayz_lib::ipc::{socket_path, IpcCommand, IpcResponse};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let command = match args[1].as_str() {
        "toggle-audio" | "audio" => IpcCommand::ToggleAudio,
        "show-kitt" | "kitt" => IpcCommand::ShowKitt,
        "show-orb" | "orb" => IpcCommand::ShowOrb,
        "show-last" | "show" => IpcCommand::ShowLastAgent,
        "hide" => IpcCommand::Hide,
        "status" => IpcCommand::GetStatus,
        "theme" => {
            if args.len() < 3 {
                eprintln!("Error: 'theme' command requires a theme name");
                eprintln!("Usage: overlayzctl theme <theme-name>");
                eprintln!("Examples:");
                eprintln!("  overlayzctl theme nord");
                eprintln!("  overlayzctl theme base16-nord");
                std::process::exit(1);
            }
            IpcCommand::SetTheme {
                name: args[2].clone(),
            }
        }
        "list-mics" | "list-microphones" => IpcCommand::ListMicrophones,
        "set-mic" | "set-microphone" => {
            if args.len() < 3 {
                eprintln!("Error: 'set-mic' command requires a device ID");
                eprintln!("Usage: overlayzctl set-mic <device-id>");
                eprintln!("Example:");
                eprintln!("  overlayzctl set-mic default");
                eprintln!("  overlayzctl set-mic <device-id-from-list-mics>");
                std::process::exit(1);
            }
            IpcCommand::SetMicrophone {
                device_id: args[2].clone(),
            }
        }
        "help" | "--help" | "-h" => {
            print_usage();
            std::process::exit(0);
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            print_usage();
            std::process::exit(1);
        }
    };

    match send_command(command) {
        Ok(response) => {
            if response.success {
                println!("{}", response.message);
                if let Some(status) = response.status {
                    println!("\nStatus:");
                    println!("  Visible: {}", status.visible);
                    println!("  Agent: {}", status.agent);
                    println!(
                        "  Audio: {}",
                        if status.audio_enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                    println!("  State: {}", status.state);
                }
                if let Some(microphones) = response.microphones {
                    println!("\nAvailable Microphones:");
                    for mic in microphones {
                        let default_marker = if mic.is_default { " (default)" } else { "" };
                        println!("  {} - {}{}", mic.device_id, mic.label, default_marker);
                    }
                }
                std::process::exit(0);
            } else {
                eprintln!("Error: {}", response.message);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to communicate with Overlayz: {}", e);
            eprintln!("Make sure the Overlayz application is running.");
            std::process::exit(1);
        }
    }
}

fn send_command(command: IpcCommand) -> Result<IpcResponse, Box<dyn std::error::Error>> {
    let socket_path = socket_path();

    let mut stream = UnixStream::connect(&socket_path).map_err(|e| {
        format!(
            "Failed to connect to socket at {}: {}",
            socket_path.display(),
            e
        )
    })?;

    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let command_json = serde_json::to_string(&command)?;
    stream.write_all(command_json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut response_data = String::new();
    stream.read_to_string(&mut response_data)?;

    let response: IpcResponse = serde_json::from_str(&response_data)?;
    Ok(response)
}

fn print_usage() {
    println!("overlayzctl - Control Overlayz application via CLI");
    println!();
    println!("USAGE:");
    println!("    overlayzctl <COMMAND> [ARGS]");
    println!();
    println!("COMMANDS:");
    println!("    toggle-audio, audio           Toggle audio recording on/off");
    println!("    show-kitt, kitt               Show K.I.T.T. agent");
    println!("    show-orb, orb                 Show Orb agent");
    println!("    show-last, show               Show last used agent");
    println!("    hide                          Hide the application window");
    println!("    status                        Get current application status");
    println!("    theme <name>                  Set the orb theme (for tinty integration)");
    println!("    list-mics, list-microphones   List available microphones");
    println!("    set-mic, set-microphone <id>  Set the active microphone by device ID");
    println!("    help, --help, -h              Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("    overlayzctl kitt                  # Show K.I.T.T.");
    println!("    overlayzctl audio                 # Toggle audio recording");
    println!("    overlayzctl status                # Check current status");
    println!("    overlayzctl hide                  # Hide the window");
    println!("    overlayzctl theme nord            # Set theme to 'nord'");
    println!("    overlayzctl list-mics             # List available microphones");
    println!("    overlayzctl set-mic default       # Set microphone to default");
}
