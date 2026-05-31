use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context;
#[cfg(windows)]
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngressEndpoint {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    #[cfg(windows)]
    WindowsNamedPipe(String),
}

impl IngressEndpoint {
    pub fn display(&self) -> IngressEndpointDisplay<'_> {
        IngressEndpointDisplay(self)
    }
}

pub struct IngressEndpointDisplay<'a>(&'a IngressEndpoint);

impl fmt::Display for IngressEndpointDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            #[cfg(unix)]
            IngressEndpoint::UnixSocket(path) => write!(formatter, "{}", path.display()),
            #[cfg(windows)]
            IngressEndpoint::WindowsNamedPipe(name) => write!(formatter, "{name}"),
        }
    }
}

pub fn pid_file_path() -> anyhow::Result<PathBuf> {
    Ok(pid_file_path_for_home(&home_dir()?))
}

pub fn app_support_dir_path() -> anyhow::Result<PathBuf> {
    platform_state_dir_path()
}

pub fn service_file_path() -> anyhow::Result<Option<PathBuf>> {
    platform_service_file_path()
}

pub fn log_file_path() -> anyhow::Result<PathBuf> {
    platform_log_file_path()
}

pub fn service_metadata_path() -> anyhow::Result<PathBuf> {
    Ok(service_metadata_path_for_home(&home_dir()?))
}

pub fn socket_file_path() -> anyhow::Result<PathBuf> {
    match ingress_endpoint()? {
        #[cfg(unix)]
        IngressEndpoint::UnixSocket(path) => Ok(path),
        #[cfg(windows)]
        IngressEndpoint::WindowsNamedPipe(name) => {
            anyhow::bail!("Windows local ingress uses named pipe `{name}`, not a socket file")
        }
    }
}

pub fn ingress_endpoint() -> anyhow::Result<IngressEndpoint> {
    platform_ingress_endpoint()
}

pub fn codex_desktop_source_state_path() -> anyhow::Result<PathBuf> {
    Ok(codex_desktop_source_state_path_for_home(&home_dir()?))
}

pub fn delivery_safety_state_path() -> anyhow::Result<PathBuf> {
    Ok(delivery_safety_state_path_for_home(&home_dir()?))
}

pub fn response_surface_ledger_path() -> anyhow::Result<PathBuf> {
    Ok(response_surface_ledger_path_for_home(&home_dir()?))
}

pub fn bridge_binding_ledger_path() -> anyhow::Result<PathBuf> {
    Ok(bridge_binding_ledger_path_for_home(&home_dir()?))
}

pub fn runtime_owner_lock_path() -> anyhow::Result<PathBuf> {
    Ok(runtime_owner_lock_path_for_home(&home_dir()?))
}

pub fn codex_sessions_dir_path() -> anyhow::Result<PathBuf> {
    Ok(codex_sessions_dir_path_for_home(&home_dir()?))
}

pub fn codex_session_index_path() -> anyhow::Result<PathBuf> {
    Ok(codex_session_index_path_for_home(&home_dir()?))
}

pub fn codex_cli_config_path() -> anyhow::Result<PathBuf> {
    Ok(codex_cli_config_path_for_home(&home_dir()?))
}

pub fn claude_code_settings_path() -> anyhow::Result<PathBuf> {
    Ok(claude_code_settings_path_for_home(&home_dir()?))
}

pub fn default_config_file_path() -> anyhow::Result<PathBuf> {
    Ok(default_config_file_path_for_home(&home_dir()?))
}

pub fn pid_file_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("agents-router.pid")
}

pub fn app_support_dir_path_for_home(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return home
        .join("Library")
        .join("Application Support")
        .join("agents-router");

    #[cfg(all(unix, not(target_os = "macos")))]
    return home.join(".local").join("state").join("agents-router");

    #[cfg(windows)]
    return home.join("AppData").join("Local").join("agents-router");
}

pub fn service_file_path_for_home(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return Some(
        home.join("Library")
            .join("LaunchAgents")
            .join("com.agents-router.service.plist"),
    );

    #[cfg(target_os = "linux")]
    return Some(
        home.join(".config")
            .join("systemd")
            .join("user")
            .join("agents-router.service"),
    );

    #[cfg(windows)]
    return None;
}

pub fn log_file_path_for_home(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return home
        .join("Library")
        .join("Logs")
        .join("agents-router")
        .join("agents-router.log");

    #[cfg(all(unix, not(target_os = "macos")))]
    return app_support_dir_path_for_home(home).join("agents-router.log");

    #[cfg(windows)]
    return app_support_dir_path_for_home(home)
        .join("logs")
        .join("agents-router.log");
}

pub fn socket_file_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("agents-router.sock")
}

pub fn service_metadata_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("service.json")
}

pub fn codex_desktop_source_state_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("codex-desktop-source-state.json")
}

pub fn delivery_safety_state_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("delivery-safety-state.json")
}

pub fn response_surface_ledger_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("response-surface-ledger.json")
}

pub fn bridge_binding_ledger_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("bridge-bindings.json")
}

pub fn runtime_owner_lock_path_for_home(home: &Path) -> PathBuf {
    app_support_dir_path_for_home(home).join("watch-owner.lock")
}

pub fn codex_sessions_dir_path_for_home(home: &Path) -> PathBuf {
    home.join(".codex").join("sessions")
}

pub fn codex_session_index_path_for_home(home: &Path) -> PathBuf {
    home.join(".codex").join("session_index.jsonl")
}

pub fn codex_cli_config_path_for_home(home: &Path) -> PathBuf {
    home.join(".codex").join("config.toml")
}

pub fn claude_code_settings_path_for_home(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

pub fn default_config_file_path_for_home(home: &Path) -> PathBuf {
    #[cfg(windows)]
    return home
        .join("AppData")
        .join("Roaming")
        .join("agents-router")
        .join("config.toml");

    #[cfg(not(windows))]
    home.join(".config")
        .join("agents-router")
        .join("config.toml")
}

fn home_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;

    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").context("HOME is not set")?;

    if home.is_empty() {
        #[cfg(windows)]
        anyhow::bail!("USERPROFILE is empty");

        #[cfg(not(windows))]
        anyhow::bail!("HOME is empty");
    }

    Ok(PathBuf::from(home))
}

fn platform_state_dir_path() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    return Ok(app_support_dir_path_for_home(&home_dir()?));

    #[cfg(target_os = "linux")]
    {
        if let Some(state_home) = std::env::var_os("XDG_STATE_HOME")
            && !state_home.is_empty()
        {
            return Ok(PathBuf::from(state_home).join("agents-router"));
        }

        return Ok(app_support_dir_path_for_home(&home_dir()?));
    }

    #[cfg(windows)]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        if local_app_data.is_empty() {
            anyhow::bail!("LOCALAPPDATA is empty");
        }
        return Ok(PathBuf::from(local_app_data).join("agents-router"));
    }
}

fn platform_service_file_path() -> anyhow::Result<Option<PathBuf>> {
    Ok(service_file_path_for_home(&home_dir()?))
}

fn platform_log_file_path() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    return Ok(log_file_path_for_home(&home_dir()?));

    #[cfg(target_os = "linux")]
    return Ok(platform_state_dir_path()?.join("agents-router.log"));

    #[cfg(windows)]
    return Ok(platform_state_dir_path()?
        .join("logs")
        .join("agents-router.log"));
}

fn platform_ingress_endpoint() -> anyhow::Result<IngressEndpoint> {
    #[cfg(unix)]
    return Ok(IngressEndpoint::UnixSocket(
        app_support_dir_path()?.join("agents-router.sock"),
    ));

    #[cfg(windows)]
    {
        let home = home_dir()?;
        let suffix = short_hash(&home.to_string_lossy());
        return Ok(IngressEndpoint::WindowsNamedPipe(format!(
            r"\\.\pipe\agents-router-{suffix}"
        )));
    }
}

#[cfg(windows)]
fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_macos_pid_file_path() {
        let home = test_home();
        let path = pid_file_path_for_home(&home);

        assert_eq!(path, expected_state_dir(&home).join("agents-router.pid"));
    }

    #[test]
    fn builds_macos_app_support_dir_path() {
        let home = test_home();
        let path = app_support_dir_path_for_home(&home);

        assert_eq!(path, expected_state_dir(&home));
    }

    #[test]
    fn builds_macos_launch_agent_plist_path() {
        let home = test_home();
        let path = service_file_path_for_home(&home);

        assert_eq!(path, expected_service_file(&home));
    }

    #[test]
    fn builds_macos_log_file_path() {
        let home = test_home();
        let path = log_file_path_for_home(&home);

        assert_eq!(path, expected_log_file(&home));
    }

    #[test]
    fn builds_macos_socket_file_path() {
        let home = test_home();
        let path = socket_file_path_for_home(&home);

        assert_eq!(path, expected_state_dir(&home).join("agents-router.sock"));
    }

    #[test]
    fn builds_service_metadata_path() {
        let home = test_home();
        let path = service_metadata_path_for_home(&home);

        assert_eq!(path, expected_state_dir(&home).join("service.json"));
    }

    #[test]
    fn builds_codex_desktop_source_state_path() {
        let home = test_home();
        let path = codex_desktop_source_state_path_for_home(&home);

        assert_eq!(
            path,
            expected_state_dir(&home).join("codex-desktop-source-state.json")
        );
    }

    #[test]
    fn builds_response_surface_ledger_path() {
        let home = test_home();
        let path = response_surface_ledger_path_for_home(&home);

        assert_eq!(
            path,
            expected_state_dir(&home).join("response-surface-ledger.json")
        );
    }

    #[test]
    fn builds_codex_sessions_dir_path() {
        let home = test_home();
        let path = codex_sessions_dir_path_for_home(&home);

        assert_eq!(path, home.join(".codex").join("sessions"));
    }

    #[test]
    fn builds_codex_session_index_path() {
        let home = test_home();
        let path = codex_session_index_path_for_home(&home);

        assert_eq!(path, home.join(".codex").join("session_index.jsonl"));
    }

    #[test]
    fn builds_codex_cli_config_path() {
        let home = test_home();
        let path = codex_cli_config_path_for_home(&home);

        assert_eq!(path, home.join(".codex").join("config.toml"));
    }

    #[test]
    fn builds_claude_code_settings_path() {
        let home = test_home();
        let path = claude_code_settings_path_for_home(&home);

        assert_eq!(path, home.join(".claude").join("settings.json"));
    }

    #[test]
    fn builds_default_config_file_path() {
        let home = test_home();
        let path = default_config_file_path_for_home(&home);

        assert_eq!(path, expected_config_file(&home));
    }

    fn test_home() -> PathBuf {
        #[cfg(windows)]
        return PathBuf::from(r"C:\Users\tester");

        #[cfg(not(windows))]
        PathBuf::from("/Users/tester")
    }

    fn expected_state_dir(home: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        return home
            .join("Library")
            .join("Application Support")
            .join("agents-router");

        #[cfg(all(unix, not(target_os = "macos")))]
        return home.join(".local").join("state").join("agents-router");

        #[cfg(windows)]
        return home.join("AppData").join("Local").join("agents-router");
    }

    fn expected_service_file(home: &Path) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        return Some(
            home.join("Library")
                .join("LaunchAgents")
                .join("com.agents-router.service.plist"),
        );

        #[cfg(target_os = "linux")]
        return Some(
            home.join(".config")
                .join("systemd")
                .join("user")
                .join("agents-router.service"),
        );

        #[cfg(windows)]
        return None;
    }

    fn expected_log_file(home: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        return home
            .join("Library")
            .join("Logs")
            .join("agents-router")
            .join("agents-router.log");

        #[cfg(all(unix, not(target_os = "macos")))]
        return expected_state_dir(home).join("agents-router.log");

        #[cfg(windows)]
        return expected_state_dir(home)
            .join("logs")
            .join("agents-router.log");
    }

    fn expected_config_file(home: &Path) -> PathBuf {
        #[cfg(windows)]
        return home
            .join("AppData")
            .join("Roaming")
            .join("agents-router")
            .join("config.toml");

        #[cfg(not(windows))]
        return home
            .join(".config")
            .join("agents-router")
            .join("config.toml");
    }
}
