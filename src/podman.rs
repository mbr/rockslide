use std::{
    env,
    fmt::Display,
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use sec::Secret;
use tempfile::tempfile;
use tokio::process::Command;
use tracing::{debug, trace};

#[derive(Debug)]
pub(crate) struct Podman {
    /// Path to the podman binary.
    podman_path: PathBuf,
    is_remote: bool,
}

impl Podman {
    /// Creates a new podman handle.
    pub(crate) fn new<P: AsRef<Path>>(podman_path: P, is_remote: bool) -> Self {
        Self {
            podman_path: podman_path.as_ref().into(),
            is_remote,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn inspect(&self, container: &str) -> Result<serde_json::Value, CommandError> {
        let mut cmd = self.mk_podman_command();
        cmd.arg("inspect");
        cmd.arg(container);
        cmd.args(["--format", "json"]);
        fetch_json(cmd).await
    }

    pub(crate) async fn login(
        &self,
        username: &str,
        password: Secret<&str>,
        registry: &str,
        tls_verify: bool,
    ) -> Result<(), CommandError> {
        let mut cmd = self.mk_podman_command();
        cmd.arg("login");
        cmd.args(["--username", username]);
        cmd.arg("--password-stdin");

        if !tls_verify {
            cmd.arg("--tls-verify=false");
        }

        cmd.arg(registry);

        let mut pw_file = tempfile()?;

        pw_file.write_all(password.reveal().as_bytes())?;
        pw_file.seek(SeekFrom::Start(0))?;

        cmd.stdin(Stdio::from(pw_file));

        checked_output(cmd).await?;

        Ok(())
    }

    pub(crate) async fn ps(&self, all: bool) -> Result<serde_json::Value, CommandError> {
        let mut cmd = self.mk_podman_command();
        cmd.arg("ps");

        if all {
            cmd.arg("--all");
        }

        cmd.args(["--format", "json"]);

        fetch_json(cmd).await
    }

    pub(crate) async fn pull(&self, image: &str) -> Result<(), CommandError> {
        // TODO: Make `--tls-verify` configurable.
        let mut cmd = self.mk_podman_command();
        cmd.arg("pull");
        cmd.arg(image);
        cmd.arg("--tls-verify=false");

        checked_output(cmd).await?;
        Ok(())
    }

    pub(crate) fn run(&self, image_url: &str) -> RunCommand {
        RunCommand {
            podman: self,
            image_url: image_url.to_owned(),
            rm: false,
            name: None,
            rmi: false,
            tls_verify: true,
            env: Vec::new(),
            publish: Vec::new(),
        }
    }

    pub(crate) async fn rm(&self, container: &str, force: bool) -> Result<Output, CommandError> {
        let mut cmd = self.mk_podman_command();

        cmd.arg("rm");

        if force {
            cmd.arg("--force");
        }

        cmd.arg(container);

        checked_output(cmd).await
    }

    fn mk_podman_command(&self) -> Command {
        let mut cmd = Command::new(&self.podman_path);

        if !self.is_remote {
            // Since we are running as a system service, we usually do not have the luxury of a
            // user-level systemd available, thus use `cgroupfs` as the cgroup manager.
            cmd.arg("--cgroup-manager=cgroupfs").kill_on_drop(true);
        }

        cmd
    }
}

pub(crate) struct RunCommand<'a> {
    podman: &'a Podman,
    env: Vec<(String, String)>,
    image_url: String,
    name: Option<String>,
    rm: bool,
    rmi: bool,
    tls_verify: bool,
    publish: Vec<String>,
}

impl<'a> RunCommand<'a> {
    pub fn env<S1: Into<String>, S2: Into<String>>(&mut self, var: S1, value: S2) -> &mut Self {
        self.env.push((var.into(), value.into()));
        self
    }

    #[inline]
    pub fn name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    #[inline]
    pub fn publish<S: Into<String>>(&mut self, publish: S) -> &mut Self {
        self.publish.push(publish.into());
        self
    }

    #[inline]
    pub(crate) fn rm(&mut self) -> &mut Self {
        self.rm = true;
        self
    }

    #[inline]
    pub(crate) fn rmi(&mut self) -> &mut Self {
        self.rmi = true;
        self
    }

    #[inline]
    pub(crate) fn tls_verify(&mut self, tls_verify: bool) -> &mut Self {
        self.tls_verify = tls_verify;
        self
    }

    #[inline]
    pub(crate) async fn execute(&self) -> Result<Output, CommandError> {
        let mut cmd = self.podman.mk_podman_command();

        cmd.arg("run");
        cmd.arg(format!("--tls-verify={}", self.tls_verify));

        // Disable health checks, since these also require a running systemd by default.
        cmd.arg("--health-cmd=none");

        cmd.arg("--detach");

        if self.rm {
            cmd.arg("--rm");
        }

        if self.rmi {
            cmd.arg("--rmi");
        }

        if let Some(ref name) = self.name {
            cmd.args(["--name", name.as_str()]);
        }

        for publish in &self.publish {
            cmd.args(["-p", publish.as_str()]);
        }

        for (key, value) in &self.env {
            cmd.args(["-e", &format!("{}={}", key, value)]);
        }

        cmd.arg(&self.image_url);

        checked_output(cmd).await
    }
}

#[derive(Debug)]
pub(crate) struct CommandError {
    err: io::Error,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
}

impl From<io::Error> for CommandError {
    fn from(value: io::Error) -> Self {
        CommandError {
            err: value,
            stdout: None,
            stderr: None,
        }
    }
}

impl Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.err.fmt(f)?;

        if let Some(ref stdout) = self.stdout {
            let text = String::from_utf8_lossy(stdout);
            f.write_str("\nstdout: ")?;
            f.write_str(&text)?;
            f.write_str("\n")?;
        }

        if let Some(ref stderr) = self.stderr {
            let text = String::from_utf8_lossy(stderr);
            f.write_str("\nstderr: ")?;
            f.write_str(&text)?;
            f.write_str("\n")?;
        }

        Ok(())
    }
}

impl std::error::Error for CommandError {}

async fn checked_output(mut cmd: Command) -> Result<Output, CommandError> {
    debug!(?cmd, "running command");
    let output = cmd.output().await?;

    if !output.status.success() {
        return Err(CommandError {
            err: io::Error::new(io::ErrorKind::Other, "non-zero exit status"),
            stdout: Some(output.stdout),
            stderr: Some(output.stderr),
        });
    }

    trace!(
        stdout = %std::str::from_utf8(&output.stdout).unwrap_or("(invalid utf8)"),
        stderr = %std::str::from_utf8(&output.stderr).unwrap_or("(invalid utf8)"),
        "command finished"
    );

    Ok(output)
}

async fn fetch_json(cmd: Command) -> Result<serde_json::Value, CommandError> {
    let output = checked_output(cmd).await?;

    trace!(raw = %String::from_utf8_lossy(&output.stdout), "parsing JSON");

    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

    Ok(parsed)
}

pub(crate) fn podman_is_remote() -> bool {
    env::var("PODMAN_IS_REMOTE").unwrap_or_default() == "true"
}
