use std::{
    env, fs,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::Serialize;

const LABEL: &str = "works.byteowlz.foxline";
const PLIST_NAME: &str = "works.byteowlz.foxline.plist";

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Run STT and the Voice Gateway in the foreground until interrupted.
    Run,
    /// Start the enabled background service for this login session.
    Start(ActionOptions),
    /// Stop the background service without disabling login startup.
    Stop(ActionOptions),
    /// Stop and start the enabled background service.
    Restart(ActionOptions),
    /// Show whether login startup is enabled and whether the service is running.
    Status(StatusOptions),
    /// Install the user LaunchAgent; --now also starts it.
    Enable(EnableOptions),
    /// Remove the user LaunchAgent; --now stops it first.
    Disable(EnableOptions),
}

#[derive(Debug, Args, Default)]
struct ActionOptions {
    /// Print the launchctl commands without executing them.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args, Default)]
struct StatusOptions {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Default)]
struct EnableOptions {
    /// Start after enabling, or stop before disabling.
    #[arg(long)]
    now: bool,
    /// Print the planned filesystem and launchctl changes without applying them.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct ServiceStatus {
    label: &'static str,
    enabled: bool,
    running: bool,
    gateway_ready: bool,
    stt_ready: bool,
    launch_agent: String,
}

pub fn execute(args: ServiceArgs) -> Result<()> {
    let manager = ServiceManager::from_environment()?;
    match args.command {
        ServiceCommand::Run => manager.run_foreground(),
        ServiceCommand::Start(options) => manager.start(options.dry_run),
        ServiceCommand::Stop(options) => manager.stop(options.dry_run),
        ServiceCommand::Restart(options) => manager.restart(options.dry_run),
        ServiceCommand::Status(options) => manager.print_status(options.json),
        ServiceCommand::Enable(options) => manager.enable(options.now, options.dry_run),
        ServiceCommand::Disable(options) => manager.disable(options.now, options.dry_run),
    }
}

struct ServiceManager {
    home: PathBuf,
    executable: PathBuf,
    repo_root: PathBuf,
}

impl ServiceManager {
    fn from_environment() -> Result<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        let executable = env::current_exe().context("resolve current foxline executable")?;
        let repo_root = resolve_repo_root()?;
        Ok(Self {
            home,
            executable,
            repo_root,
        })
    }

    #[cfg(test)]
    fn new(home: PathBuf, executable: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            home,
            executable,
            repo_root,
        }
    }

    fn plist_path(&self) -> PathBuf {
        self.home.join("Library/LaunchAgents").join(PLIST_NAME)
    }

    fn domain_target(&self) -> Result<String> {
        let output = Command::new("id").arg("-u").output().context("run id -u")?;
        if !output.status.success() {
            bail!("id -u failed with {}", output.status);
        }
        let uid = String::from_utf8(output.stdout)
            .context("id -u returned invalid UTF-8")?
            .trim()
            .to_string();
        Ok(format!("gui/{uid}"))
    }

    fn service_target(&self) -> Result<String> {
        Ok(format!("{}/{}", self.domain_target()?, LABEL))
    }

    fn run_foreground(&self) -> Result<()> {
        let script = self.repo_root.join("scripts/start-services.sh");
        if !script.is_file() {
            bail!(
                "STT launcher not found at {}; set FOXLINE_ROOT to the Foxline installation root",
                script.display()
            );
        }

        println!("Starting Foxline STT and Voice Gateway...");
        let mut stt = grouped_command("bash")
            .arg(&script)
            .args(["--no-transcript-server", "--foreground"])
            .current_dir(&self.repo_root)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start Parakeet/Silero STT")?;

        let mut gateway = self
            .spawn_gateway()
            .inspect_err(|_| terminate_child_group(&mut stt))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&stopping);
        ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))
            .context("install service signal handler")?;

        let result = loop {
            if stopping.load(Ordering::SeqCst) {
                break Ok(());
            }
            if let Some(status) = stt.try_wait().context("poll STT process")? {
                break Err(anyhow::anyhow!("STT exited unexpectedly with {status}"));
            }
            if let Some(status) = gateway.try_wait().context("poll gateway process")? {
                break Err(anyhow::anyhow!(
                    "Voice Gateway exited unexpectedly with {status}"
                ));
            }
            thread::sleep(Duration::from_millis(100));
        };

        terminate_child_group(&mut gateway);
        terminate_child_group(&mut stt);
        result
    }

    fn spawn_gateway(&self) -> Result<Child> {
        if let Some(command) = env::var_os("FOXLINE_GATEWAY_COMMAND") {
            return grouped_command("bash")
                .args(["-lc", &command.to_string_lossy()])
                .current_dir(&self.repo_root)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .context("start configured Voice Gateway command");
        }

        let sibling = self
            .executable
            .parent()
            .unwrap_or(Path::new("."))
            .join("foxline-voice-gateway");
        if sibling.is_file() {
            grouped_command(sibling)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .context("start installed Voice Gateway")
        } else {
            grouped_command("cargo")
                .args(["run", "-p", "foxline-voice-gateway", "--"])
                .current_dir(&self.repo_root)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .context("start Voice Gateway through Cargo")
        }
    }

    fn enable(&self, now: bool, dry_run: bool) -> Result<()> {
        let path = self.plist_path();
        if dry_run {
            println!("Would write {}", path.display());
            if now {
                self.start(true)?;
            }
            return Ok(());
        }
        let parent = path.parent().context("LaunchAgents path has no parent")?;
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let state_dir = self.home.join(".local/state/foxline");
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("create {}", state_dir.display()))?;
        let rendered = self.render_plist();
        if path.exists() {
            let existing = fs::read_to_string(&path)
                .with_context(|| format!("read existing {}", path.display()))?;
            if existing != rendered {
                bail!(
                    "{} already exists with different content; move it aside before enabling Foxline",
                    path.display()
                );
            }
        } else {
            fs::write(&path, rendered).with_context(|| format!("write {}", path.display()))?;
        }
        println!("Enabled Foxline at login: {}", path.display());
        if now {
            self.start(false)?;
        }
        Ok(())
    }

    fn disable(&self, now: bool, dry_run: bool) -> Result<()> {
        let path = self.plist_path();
        if now {
            self.stop(dry_run)?;
        }
        if dry_run {
            println!("Would remove {}", path.display());
            return Ok(());
        }
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
        println!("Disabled Foxline login startup.");
        Ok(())
    }

    fn start(&self, dry_run: bool) -> Result<()> {
        let path = self.plist_path();
        if !path.is_file() && !dry_run {
            bail!(
                "Foxline is not enabled. Run `foxline service enable --now` or `foxline service run`."
            );
        }
        let domain = self.domain_target()?;
        let path_text = path.to_string_lossy();
        let bootstrap = ["bootstrap", domain.as_str(), path_text.as_ref()];
        if self.is_loaded() {
            self.launchctl(&["kickstart", "-k", &self.service_target()?], dry_run)
        } else {
            self.launchctl(&bootstrap, dry_run)
        }
    }

    fn stop(&self, dry_run: bool) -> Result<()> {
        if !self.plist_path().is_file() && !dry_run {
            bail!("Foxline is not enabled; there is no background service to stop.");
        }
        if !dry_run && !self.is_loaded() {
            println!("Foxline service is already stopped.");
            return Ok(());
        }
        self.launchctl(&["bootout", &self.service_target()?], dry_run)
    }

    fn restart(&self, dry_run: bool) -> Result<()> {
        if !self.plist_path().is_file() && !dry_run {
            bail!("Foxline is not enabled. Run `foxline service enable --now` first.");
        }
        if self.is_loaded() || dry_run {
            self.stop(dry_run)?;
        }
        self.start(dry_run)
    }

    fn print_status(&self, json: bool) -> Result<()> {
        let status = ServiceStatus {
            label: LABEL,
            enabled: self.plist_path().is_file(),
            running: self.is_running(),
            gateway_ready: tcp_ready("127.0.0.1:8780"),
            stt_ready: tcp_ready("127.0.0.1:8796"),
            launch_agent: self.plist_path().display().to_string(),
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            println!(
                "Autostart: {}\nService:   {}\nGateway:   {}\nSTT:       {}\nLaunchAgent: {}",
                if status.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if status.running { "running" } else { "stopped" },
                if status.gateway_ready {
                    "ready"
                } else {
                    "unavailable"
                },
                if status.stt_ready {
                    "ready"
                } else {
                    "unavailable"
                },
                status.launch_agent
            );
        }
        Ok(())
    }

    fn launchctl_description(&self) -> Option<String> {
        let target = self.service_target().ok()?;
        let output = Command::new("launchctl")
            .args(["print", &target])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn is_loaded(&self) -> bool {
        self.launchctl_description().is_some()
    }

    fn is_running(&self) -> bool {
        self.launchctl_description()
            .is_some_and(|description| launchctl_description_is_running(&description))
    }

    fn launchctl(&self, args: &[&str], dry_run: bool) -> Result<()> {
        if dry_run {
            println!("Would run: launchctl {}", args.join(" "));
            return Ok(());
        }
        let status = Command::new("launchctl")
            .args(args)
            .status()
            .with_context(|| format!("run launchctl {}", args.join(" ")))?;
        if !status.success() {
            bail!("launchctl {} failed with {status}", args.join(" "));
        }
        Ok(())
    }

    fn render_plist(&self) -> String {
        let executable = xml_escape(&self.executable.to_string_lossy());
        let root = xml_escape(&self.repo_root.to_string_lossy());
        let path = xml_escape(&env::var("PATH").unwrap_or_else(|_| {
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
        }));
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{executable}</string>
    <string>service</string>
    <string>run</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{root}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>FOXLINE_ROOT</key>
    <string>{root}</string>
    <key>PATH</key>
    <string>{path}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>{home}/.local/state/foxline/service.log</string>
  <key>StandardErrorPath</key>
  <string>{home}/.local/state/foxline/service.log</string>
</dict>
</plist>
"#,
            home = xml_escape(&self.home.to_string_lossy())
        )
    }
}

fn resolve_repo_root() -> Result<PathBuf> {
    if let Some(root) = env::var_os("FOXLINE_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let mut current = env::current_dir().context("resolve current directory")?;
    loop {
        if current.join("scripts/start-services.sh").is_file()
            && current.join("Cargo.toml").is_file()
        {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    bail!("could not find Foxline installation root; set FOXLINE_ROOT")
}

#[cfg(unix)]
fn grouped_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command.process_group(0);
    command
}

#[cfg(not(unix))]
fn grouped_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    Command::new(program)
}

fn terminate_child_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", child.id())])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn tcp_ready(address: &str) -> bool {
    address.parse::<SocketAddr>().is_ok_and(|address| {
        TcpStream::connect_timeout(&address, Duration::from_millis(150)).is_ok()
    })
}

fn launchctl_description_is_running(description: &str) -> bool {
    description.lines().any(|line| {
        let line = line.trim();
        line == "state = running" || line.starts_with("pid = ")
    })
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn manager(temp: &TempDir) -> ServiceManager {
        let root = temp.path().join("repo");
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(
            root.join("scripts/start-services.sh"),
            "#!/usr/bin/env bash\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        ServiceManager::new(
            temp.path().join("home"),
            temp.path().join("bin/foxline"),
            root,
        )
    }

    #[test]
    fn launch_agent_uses_service_run_and_is_xml_escaped() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        let plist = manager.render_plist();
        assert!(plist.contains("works.byteowlz.foxline"));
        assert!(plist.contains("<string>service</string>"));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains("<key>FOXLINE_ROOT</key>"));
        assert!(!plist.contains("tmux"));
    }

    #[test]
    fn enable_and_disable_only_touch_isolated_home() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        manager.enable(false, false).unwrap();
        let path = manager.plist_path();
        assert!(path.starts_with(temp.path()));
        assert!(path.is_file());
        manager.disable(false, false).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn enable_refuses_to_overwrite_a_modified_launch_agent() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        let path = manager.plist_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "user-managed").unwrap();
        let error = manager.enable(false, false).unwrap_err().to_string();
        assert!(error.contains("different content"));
        assert_eq!(fs::read_to_string(path).unwrap(), "user-managed");
    }

    #[test]
    fn dry_run_does_not_write_launch_agent() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        manager.enable(false, true).unwrap();
        assert!(!manager.plist_path().exists());
        manager.disable(false, true).unwrap();
    }

    #[test]
    fn xml_escaping_handles_launchd_metacharacters() {
        assert_eq!(xml_escape("a&<b>\"'"), "a&amp;&lt;b&gt;&quot;&apos;");
    }

    #[test]
    fn launchctl_status_distinguishes_loaded_from_running() {
        assert!(launchctl_description_is_running(
            "state = running\npid = 123\n"
        ));
        assert!(!launchctl_description_is_running(
            "state = exited\nlast exit code = 1\n"
        ));
    }
}
