use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::paths::service_file_path;

pub const LAUNCH_AGENT_LABEL: &str = "com.agents-router.service";
pub const SYSTEMD_USER_UNIT_NAME: &str = "agents-router.service";
pub const WINDOWS_TASK_NAME: &str = r"\AgentsRouter";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDefinition {
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub working_dir: PathBuf,
    pub log_file: PathBuf,
    pub home: String,
    pub env_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStartOutcome {
    AlreadyRunning,
    Started,
    UpdatedAndStarted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStopOutcome {
    NotInstalled,
    AlreadyStopped,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub platform: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceMetadata {
    pub binary_path: String,
    pub config_path: String,
    pub log_file: String,
    pub installed_at: String,
}

impl ServiceMetadata {
    #[cfg(windows)]
    fn matches_definition(&self, definition: &ServiceDefinition) -> bool {
        self.binary_path == definition.binary_path.display().to_string()
            && self.config_path == definition.config_path.display().to_string()
            && self.log_file == definition.log_file.display().to_string()
    }
}

pub enum PlatformServiceManager {
    #[cfg(target_os = "macos")]
    LaunchAgent(LaunchAgentManager<SystemLaunchctl>),
    #[cfg(target_os = "linux")]
    SystemdUser(SystemdUserManager),
    #[cfg(windows)]
    WindowsTaskScheduler(WindowsTaskSchedulerManager),
}

impl PlatformServiceManager {
    pub fn system() -> anyhow::Result<Self> {
        #[cfg(target_os = "macos")]
        return Ok(Self::LaunchAgent(LaunchAgentManager::system()?));

        #[cfg(target_os = "linux")]
        return Ok(Self::SystemdUser(SystemdUserManager::system()?));

        #[cfg(windows)]
        return Ok(Self::WindowsTaskScheduler(
            WindowsTaskSchedulerManager::system()?,
        ));
    }

    pub fn install_or_update(
        &self,
        definition: &ServiceDefinition,
        metadata_path: &Path,
    ) -> anyhow::Result<ServiceStartOutcome> {
        match self {
            #[cfg(target_os = "macos")]
            Self::LaunchAgent(manager) => {
                let plist_path = service_file_path()?.context("missing LaunchAgent plist path")?;
                manager.install_or_update(definition, &plist_path, metadata_path)
            }
            #[cfg(target_os = "linux")]
            Self::SystemdUser(manager) => {
                let unit_path = service_file_path()?.context("missing systemd user unit path")?;
                manager.install_or_update(definition, &unit_path, metadata_path)
            }
            #[cfg(windows)]
            Self::WindowsTaskScheduler(manager) => {
                manager.install_or_update(definition, metadata_path)
            }
        }
    }

    pub fn stop(&self) -> anyhow::Result<ServiceStopOutcome> {
        match self {
            #[cfg(target_os = "macos")]
            Self::LaunchAgent(manager) => {
                let plist_path = service_file_path()?.context("missing LaunchAgent plist path")?;
                manager.stop(&plist_path)
            }
            #[cfg(target_os = "linux")]
            Self::SystemdUser(manager) => {
                let unit_path = service_file_path()?.context("missing systemd user unit path")?;
                manager.stop(&unit_path)
            }
            #[cfg(windows)]
            Self::WindowsTaskScheduler(manager) => manager.stop(),
        }
    }

    pub fn status(&self) -> anyhow::Result<ServiceStatus> {
        match self {
            #[cfg(target_os = "macos")]
            Self::LaunchAgent(manager) => {
                let plist_path = service_file_path()?.context("missing LaunchAgent plist path")?;
                manager.status(&plist_path)
            }
            #[cfg(target_os = "linux")]
            Self::SystemdUser(manager) => {
                let unit_path = service_file_path()?.context("missing systemd user unit path")?;
                manager.status(&unit_path)
            }
            #[cfg(windows)]
            Self::WindowsTaskScheduler(manager) => manager.status(),
        }
    }

    pub fn uninstall(&self) -> anyhow::Result<()> {
        match self {
            #[cfg(target_os = "macos")]
            Self::LaunchAgent(manager) => {
                let plist_path = service_file_path()?.context("missing LaunchAgent plist path")?;
                manager.stop(&plist_path)?;
                remove_file_if_exists(&plist_path)?;
                Ok(())
            }
            #[cfg(target_os = "linux")]
            Self::SystemdUser(manager) => {
                let unit_path = service_file_path()?.context("missing systemd user unit path")?;
                manager.uninstall(&unit_path)
            }
            #[cfg(windows)]
            Self::WindowsTaskScheduler(manager) => manager.uninstall(),
        }
    }

    pub fn service_file_path(&self) -> anyhow::Result<Option<PathBuf>> {
        service_file_path()
    }
}

pub trait LaunchctlRunner {
    fn run(&self, args: &[String]) -> anyhow::Result<String>;
}

pub struct SystemLaunchctl;

impl LaunchctlRunner for SystemLaunchctl {
    fn run(&self, args: &[String]) -> anyhow::Result<String> {
        ensure_macos()?;

        let output = Command::new("launchctl")
            .args(args)
            .output()
            .with_context(|| format!("failed to run launchctl {}", args.join(" ")))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr).trim().to_string();

        if output.status.success() {
            Ok(combined)
        } else {
            anyhow::bail!("launchctl {} failed: {}", args.join(" "), combined);
        }
    }
}

pub struct LaunchAgentManager<R> {
    runner: R,
    uid: u32,
}

impl LaunchAgentManager<SystemLaunchctl> {
    pub fn system() -> anyhow::Result<Self> {
        ensure_macos()?;
        Ok(Self {
            runner: SystemLaunchctl,
            uid: current_uid()?,
        })
    }
}

impl<R: LaunchctlRunner> LaunchAgentManager<R> {
    pub fn with_uid(runner: R, uid: u32) -> Self {
        Self { runner, uid }
    }

    pub fn install_or_update(
        &self,
        definition: &ServiceDefinition,
        plist_path: &Path,
        metadata_path: &Path,
    ) -> anyhow::Result<ServiceStartOutcome> {
        if let Some(parent) = definition.log_file.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create log directory `{}`", parent.display())
            })?;
        }
        fs::create_dir_all(&definition.working_dir).with_context(|| {
            format!(
                "failed to create service working directory `{}`",
                definition.working_dir.display()
            )
        })?;

        let desired_plist = build_plist(definition);
        let installed = plist_path.exists();
        let current_plist = if installed {
            Some(fs::read_to_string(plist_path).with_context(|| {
                format!(
                    "failed to read LaunchAgent plist `{}`",
                    plist_path.display()
                )
            })?)
        } else {
            None
        };
        let needs_update = current_plist.as_deref() != Some(desired_plist.as_str());
        let status = self.status(plist_path)?;

        if installed && !needs_update && status.running {
            save_metadata(metadata_path, definition)?;
            return Ok(ServiceStartOutcome::AlreadyRunning);
        }

        if needs_update {
            if status.running {
                self.bootout_targets()?;
            } else {
                let _ = self.bootout_targets();
            }
            write_plist(plist_path, &desired_plist)?;
        } else if !installed {
            write_plist(plist_path, &desired_plist)?;
        }

        let domain = self.preferred_domain();
        let target = launchd_target(&domain);
        let plist = plist_path.display().to_string();
        let bootstrap_args = vec!["bootstrap".to_string(), domain, plist];
        if self.runner.run(&bootstrap_args).is_err() {
            let kickstart_args = vec!["kickstart".to_string(), "-kp".to_string(), target.clone()];
            self.runner.run(&kickstart_args)?;
        } else {
            let kickstart_args = vec!["kickstart".to_string(), "-kp".to_string(), target];
            self.runner.run(&kickstart_args)?;
        }

        save_metadata(metadata_path, definition)?;

        if installed && needs_update {
            Ok(ServiceStartOutcome::UpdatedAndStarted)
        } else {
            Ok(ServiceStartOutcome::Started)
        }
    }

    pub fn stop(&self, plist_path: &Path) -> anyhow::Result<ServiceStopOutcome> {
        let status = self.status(plist_path)?;
        if !status.installed {
            return Ok(ServiceStopOutcome::NotInstalled);
        }

        if status.running {
            self.bootout_targets()?;
            let next_status = self.status(plist_path)?;
            if next_status.running {
                anyhow::bail!("LaunchAgent remained running after launchctl bootout");
            }
            Ok(ServiceStopOutcome::Stopped)
        } else {
            let _ = self.bootout_targets();
            Ok(ServiceStopOutcome::AlreadyStopped)
        }
    }

    pub fn status(&self, plist_path: &Path) -> anyhow::Result<ServiceStatus> {
        let mut status = ServiceStatus {
            installed: plist_path.exists(),
            running: false,
            pid: None,
            platform: "launchd",
        };

        if !status.installed {
            return Ok(status);
        }

        for domain in self.launchd_domains() {
            let target = launchd_target(&domain);
            let args = vec!["print".to_string(), target];
            if let Ok(output) = self.runner.run(&args) {
                let parsed = parse_launchctl_print(&output);
                status.running = parsed.running;
                status.pid = parsed.pid;
                return Ok(status);
            }
        }

        Ok(status)
    }

    fn preferred_domain(&self) -> String {
        let gui_domain = self.gui_domain();
        let args = vec!["print".to_string(), gui_domain.clone()];
        if self.runner.run(&args).is_ok() {
            gui_domain
        } else {
            self.user_domain()
        }
    }

    fn launchd_domains(&self) -> Vec<String> {
        let preferred = self.preferred_domain();
        let gui_domain = self.gui_domain();
        let user_domain = self.user_domain();
        if preferred == gui_domain {
            vec![gui_domain, user_domain]
        } else {
            vec![user_domain, gui_domain]
        }
    }

    fn bootout_targets(&self) -> anyhow::Result<()> {
        let mut failed = Vec::new();
        let mut succeeded = false;
        for domain in self.launchd_domains() {
            let target = launchd_target(&domain);
            let args = vec!["bootout".to_string(), target.clone()];
            match self.runner.run(&args) {
                Ok(_) => succeeded = true,
                Err(error) => {
                    failed.push(format!("launchctl bootout {target} failed: {error}"));
                }
            }
        }
        if succeeded {
            Ok(())
        } else {
            anyhow::bail!("{}", failed.join("; "))
        }
    }

    fn gui_domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn user_domain(&self) -> String {
        format!("user/{}", self.uid)
    }
}

#[cfg(target_os = "linux")]
pub struct SystemdUserManager;

#[cfg(target_os = "linux")]
impl SystemdUserManager {
    pub fn system() -> anyhow::Result<Self> {
        ensure_linux()?;
        Ok(Self)
    }

    pub fn install_or_update(
        &self,
        definition: &ServiceDefinition,
        unit_path: &Path,
        metadata_path: &Path,
    ) -> anyhow::Result<ServiceStartOutcome> {
        if let Some(parent) = definition.log_file.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create log directory `{}`", parent.display())
            })?;
        }

        let desired_unit = build_systemd_unit(definition);
        let installed = unit_path.exists();
        let current_unit = if installed {
            Some(fs::read_to_string(unit_path).with_context(|| {
                format!("failed to read systemd user unit `{}`", unit_path.display())
            })?)
        } else {
            None
        };
        let needs_update = current_unit.as_deref() != Some(desired_unit.as_str());
        let status = self.status(unit_path)?;

        if installed && !needs_update && status.running {
            save_metadata(metadata_path, definition)?;
            return Ok(ServiceStartOutcome::AlreadyRunning);
        }

        if needs_update || !installed {
            write_text_file(unit_path, &desired_unit, "systemd user unit")?;
            run_systemctl(&["--user", "daemon-reload"])?;
        }

        if installed && needs_update && status.running {
            run_systemctl(&["--user", "restart", SYSTEMD_USER_UNIT_NAME])?;
        } else {
            run_systemctl(&["--user", "enable", "--now", SYSTEMD_USER_UNIT_NAME])?;
        }

        save_metadata(metadata_path, definition)?;

        if installed && needs_update {
            Ok(ServiceStartOutcome::UpdatedAndStarted)
        } else {
            Ok(ServiceStartOutcome::Started)
        }
    }

    pub fn stop(&self, unit_path: &Path) -> anyhow::Result<ServiceStopOutcome> {
        let status = self.status(unit_path)?;
        if !status.installed {
            return Ok(ServiceStopOutcome::NotInstalled);
        }

        if status.running {
            run_systemctl(&["--user", "stop", SYSTEMD_USER_UNIT_NAME])?;
            Ok(ServiceStopOutcome::Stopped)
        } else {
            Ok(ServiceStopOutcome::AlreadyStopped)
        }
    }

    pub fn status(&self, unit_path: &Path) -> anyhow::Result<ServiceStatus> {
        let installed = unit_path.exists();
        let running = installed
            && Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", SYSTEMD_USER_UNIT_NAME])
                .status()
                .context("failed to run systemctl --user is-active")?
                .success();
        let pid = if installed {
            systemd_main_pid().ok().flatten()
        } else {
            None
        };

        Ok(ServiceStatus {
            installed,
            running,
            pid,
            platform: "systemd user",
        })
    }

    pub fn uninstall(&self, unit_path: &Path) -> anyhow::Result<()> {
        let installed = unit_path.exists();
        if installed {
            let _ = run_systemctl(&["--user", "stop", SYSTEMD_USER_UNIT_NAME]);
            let _ = run_systemctl(&["--user", "disable", SYSTEMD_USER_UNIT_NAME]);
            remove_file_if_exists(unit_path)?;
            run_systemctl(&["--user", "daemon-reload"])?;
        }

        Ok(())
    }
}

#[cfg(windows)]
pub struct WindowsTaskSchedulerManager;

#[cfg(windows)]
impl WindowsTaskSchedulerManager {
    pub fn system() -> anyhow::Result<Self> {
        ensure_windows()?;
        Ok(Self)
    }

    pub fn install_or_update(
        &self,
        definition: &ServiceDefinition,
        metadata_path: &Path,
    ) -> anyhow::Result<ServiceStartOutcome> {
        if let Some(parent) = definition.log_file.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create log directory `{}`", parent.display())
            })?;
        }

        let installed = windows_task_installed()?;
        let status = self.status()?;
        let needs_update = load_metadata(metadata_path)
            .map(|metadata| !metadata.matches_definition(definition))
            .unwrap_or(true);

        if installed && !needs_update && status.running {
            save_metadata(metadata_path, definition)?;
            return Ok(ServiceStartOutcome::AlreadyRunning);
        }

        if status.running {
            let _ = run_schtasks(&["/End", "/TN", WINDOWS_TASK_NAME]);
        }

        let task_command = windows_task_command(definition);
        run_schtasks(&[
            "/Create",
            "/TN",
            WINDOWS_TASK_NAME,
            "/SC",
            "ONLOGON",
            "/TR",
            &task_command,
            "/RL",
            "LIMITED",
            "/F",
        ])?;
        run_schtasks(&["/Run", "/TN", WINDOWS_TASK_NAME])?;
        save_metadata(metadata_path, definition)?;

        if installed && needs_update {
            Ok(ServiceStartOutcome::UpdatedAndStarted)
        } else {
            Ok(ServiceStartOutcome::Started)
        }
    }

    pub fn stop(&self) -> anyhow::Result<ServiceStopOutcome> {
        let status = self.status()?;
        if !status.installed {
            return Ok(ServiceStopOutcome::NotInstalled);
        }

        if status.running {
            run_schtasks(&["/End", "/TN", WINDOWS_TASK_NAME])?;
            Ok(ServiceStopOutcome::Stopped)
        } else {
            Ok(ServiceStopOutcome::AlreadyStopped)
        }
    }

    pub fn status(&self) -> anyhow::Result<ServiceStatus> {
        let output = Command::new("schtasks.exe")
            .args(["/Query", "/TN", WINDOWS_TASK_NAME, "/FO", "LIST", "/V"])
            .output()
            .context("failed to run schtasks.exe /Query")?;

        if !output.status.success() {
            return Ok(ServiceStatus {
                installed: false,
                running: false,
                pid: None,
                platform: "Task Scheduler",
            });
        }

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let running = combined
            .lines()
            .map(str::trim)
            .any(|line| line.eq_ignore_ascii_case("Status: Running"));

        Ok(ServiceStatus {
            installed: true,
            running,
            pid: None,
            platform: "Task Scheduler",
        })
    }

    pub fn uninstall(&self) -> anyhow::Result<()> {
        if windows_task_installed()? {
            let _ = run_schtasks(&["/End", "/TN", WINDOWS_TASK_NAME]);
            run_schtasks(&["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"])?;
        }

        Ok(())
    }
}

pub fn build_plist(definition: &ServiceDefinition) -> String {
    let binary = xml_escape(&definition.binary_path.display().to_string());
    let config = xml_escape(&definition.config_path.display().to_string());
    let working_dir = xml_escape(&definition.working_dir.display().to_string());
    let log_file = xml_escape(&definition.log_file.display().to_string());
    let home = xml_escape(&definition.home);
    let env_path = xml_escape(&definition.env_path);

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{binary}</string>
		<string>watch</string>
		<string>--config</string>
		<string>{config}</string>
	</array>
	<key>WorkingDirectory</key>
	<string>{working_dir}</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<array>
		<string>Aqua</string>
		<string>Background</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>HOME</key>
		<string>{home}</string>
		<key>PATH</key>
		<string>{env_path}</string>
	</dict>
	<key>StandardOutPath</key>
	<string>{log_file}</string>
	<key>StandardErrorPath</key>
	<string>{log_file}</string>
</dict>
</plist>
"#,
        label = LAUNCH_AGENT_LABEL,
        binary = binary,
        config = config,
        working_dir = working_dir,
        log_file = log_file,
        home = home,
        env_path = env_path,
    )
}

#[cfg(target_os = "linux")]
pub fn build_systemd_unit(definition: &ServiceDefinition) -> String {
    let binary = systemd_quote(&definition.binary_path.display().to_string());
    let config = systemd_quote(&definition.config_path.display().to_string());
    let working_dir = systemd_quote(&definition.working_dir.display().to_string());
    let log_file = definition.log_file.display().to_string();
    let home = systemd_escape_value(&definition.home);
    let env_path = systemd_escape_value(&definition.env_path);

    format!(
        r#"[Unit]
Description=Agents Router local notification service
After=default.target

[Service]
Type=simple
ExecStart={binary} watch --config {config}
WorkingDirectory={working_dir}
Environment="HOME={home}"
Environment="PATH={env_path}"
Restart=always
RestartSec=2
StandardOutput=append:{log_file}
StandardError=append:{log_file}

[Install]
WantedBy=default.target
"#
    )
}

#[cfg(target_os = "linux")]
fn systemd_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", systemd_escape_value(value))
    }
}

#[cfg(target_os = "linux")]
fn systemd_escape_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "linux")]
fn run_systemctl(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .with_context(|| format!("failed to run systemctl {}", args.join(" ")))?;
    command_output_or_error("systemctl", args, output)
}

#[cfg(target_os = "linux")]
fn systemd_main_pid() -> anyhow::Result<Option<u32>> {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            SYSTEMD_USER_UNIT_NAME,
            "--property",
            "MainPID",
            "--value",
        ])
        .output()
        .context("failed to run systemctl --user show MainPID")?;
    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let pid = raw.trim().parse::<u32>().ok().filter(|pid| *pid > 0);
    Ok(pid)
}

#[cfg(windows)]
fn run_schtasks(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("schtasks.exe")
        .args(args)
        .output()
        .with_context(|| format!("failed to run schtasks.exe {}", args.join(" ")))?;
    command_output_or_error("schtasks.exe", args, output)
}

#[cfg(windows)]
fn windows_task_installed() -> anyhow::Result<bool> {
    let output = Command::new("schtasks.exe")
        .args(["/Query", "/TN", WINDOWS_TASK_NAME])
        .output()
        .context("failed to run schtasks.exe /Query")?;
    Ok(output.status.success())
}

#[cfg(windows)]
fn windows_task_command(definition: &ServiceDefinition) -> String {
    format!(
        "\"{}\" watch --config \"{}\"",
        definition.binary_path.display(),
        definition.config_path.display()
    )
}

#[cfg(any(target_os = "linux", windows))]
fn command_output_or_error(
    program: &str,
    args: &[&str],
    output: std::process::Output,
) -> anyhow::Result<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).trim().to_string();

    if output.status.success() {
        Ok(combined)
    } else {
        anyhow::bail!("{} {} failed: {}", program, args.join(" "), combined)
    }
}

pub fn load_metadata(path: &Path) -> anyhow::Result<ServiceMetadata> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read service metadata `{}`", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse service metadata `{}`", path.display()))
}

pub fn save_metadata(path: &Path, definition: &ServiceDefinition) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create service metadata directory `{}`",
                parent.display()
            )
        })?;
    }

    let metadata = ServiceMetadata {
        binary_path: definition.binary_path.display().to_string(),
        config_path: definition.config_path.display().to_string(),
        log_file: definition.log_file.display().to_string(),
        installed_at: Utc::now().to_rfc3339(),
    };
    let raw = serde_json::to_string_pretty(&metadata).context("failed to serialize metadata")?;
    fs::write(path, raw)
        .with_context(|| format!("failed to write service metadata `{}`", path.display()))
}

fn write_plist(path: &Path, plist: &str) -> anyhow::Result<()> {
    write_text_file(path, plist, "LaunchAgent plist")
}

fn write_text_file(path: &Path, content: &str, label: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create {label} directory `{}`", parent.display(),)
        })?;
    }
    fs::write(path, content)
        .with_context(|| format!("failed to write {label} `{}`", path.display()))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_file_if_exists(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove service file `{}`", path.display()))?;
    }

    Ok(())
}

struct ParsedLaunchctlPrint {
    running: bool,
    pid: Option<u32>,
}

fn parse_launchctl_print(output: &str) -> ParsedLaunchctlPrint {
    let mut parsed = ParsedLaunchctlPrint {
        running: false,
        pid: None,
    };

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(raw_pid) = trimmed.strip_prefix("pid = ")
            && let Ok(pid) = raw_pid.parse::<u32>()
            && pid > 0
        {
            parsed.pid = Some(pid);
            parsed.running = true;
        }
        if trimmed.contains("state = running") {
            parsed.running = true;
        }
    }

    parsed
}

fn launchd_target(domain: &str) -> String {
    format!("{domain}/{LAUNCH_AGENT_LABEL}")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn ensure_macos() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        anyhow::bail!("LaunchAgent service management is only supported on macOS")
    }
}

#[cfg(target_os = "linux")]
fn ensure_linux() -> anyhow::Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        anyhow::bail!("systemd user service management is only supported on Linux")
    }
}

#[cfg(windows)]
fn ensure_windows() -> anyhow::Result<()> {
    if cfg!(windows) {
        Ok(())
    } else {
        anyhow::bail!("Task Scheduler service management is only supported on Windows")
    }
}

fn current_uid() -> anyhow::Result<u32> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to detect current user id")?;
    if !output.status.success() {
        anyhow::bail!("failed to detect current user id");
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.trim()
        .parse::<u32>()
        .context("failed to parse current user id")
}

#[cfg(test)]
mod tests;
