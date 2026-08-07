use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_AGENT_PORT: u16 = 17319;
const REL_AGENT_PORT_ENV: &str = "REL_AGENT_PORT";

pub(crate) fn ensure_agent_running() -> Result<(), String> {
    let port = agent_port();
    if agent_is_healthy(port) {
        return Ok(());
    }

    launch_app()?;
    wait_for_agent(port, Duration::from_secs(8))
}

fn agent_port() -> u16 {
    env::var(REL_AGENT_PORT_ENV)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_AGENT_PORT)
}

fn agent_is_healthy(port: u16) -> bool {
    let response = ureq::get(&format!("http://127.0.0.1:{port}/v1/health"))
        .timeout(Duration::from_millis(300))
        .call();
    let Ok(response) = response else {
        return false;
    };
    response
        .into_string()
        .map(|body| body.contains("\"status\":\"ok\""))
        .unwrap_or(false)
}

fn wait_for_agent(port: u16, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if agent_is_healthy(port) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(format!(
        "Rel agent did not become ready on 127.0.0.1:{port}"
    ))
}

fn launch_app() -> Result<(), String> {
    let Some(app_path) = app_path() else {
        return Err(
            "Rel.app was not found. Install Rel in /Applications from https://rel.me.".to_string(),
        );
    };
    let status = Command::new("/usr/bin/open")
        .args(["-gj"])
        .arg(&app_path)
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("Could not launch {}: {error}", app_path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not launch Rel.app at {}",
            app_path.display()
        ))
    }
}

fn app_path() -> Option<PathBuf> {
    if let Ok(executable) = env::current_exe() {
        if let Some(app) = app_bundle_ancestor(&executable) {
            return Some(app);
        }
    }

    let installed_app = PathBuf::from("/Applications/Rel.app");
    installed_app.is_dir().then_some(installed_app)
}

fn app_bundle_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.extension() == Some(OsStr::new("app")))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_app_bundle_ancestor() {
        assert_eq!(
            app_bundle_ancestor(Path::new("/Applications/Rel.app/Contents/Resources/rel")),
            Some(PathBuf::from("/Applications/Rel.app"))
        );
        assert_eq!(app_bundle_ancestor(Path::new("/usr/local/bin/rel")), None);
    }
}
