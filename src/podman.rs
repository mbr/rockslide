use std::{
    fmt::Display,
    io,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tracing::debug;

#[derive(Debug)]
pub(crate) struct Podman {
    /// Path to the podman binary.
    podman_path: PathBuf,
}

impl Podman {
    /// Creates a new podman handle.
    pub(crate) fn new<P: AsRef<Path>>(podman_path: P) -> Self {
        Self {
            podman_path: podman_path.as_ref().into(),
        }
    }

    pub(crate) fn run(&self, image_url: &str) -> StartCommand {
        StartCommand {
            podman: &self,
            image_url: image_url.to_owned(),
            rm: false,
            name: None,
            rmi: false,
            tls_verify: true,
        }
    }

    fn mk_podman_command(&self) -> Command {
        Command::new(&self.podman_path)
    }
}

pub(crate) struct StartCommand<'a> {
    podman: &'a Podman,
    image_url: String,
    name: Option<String>,
    rm: bool,
    rmi: bool,
    tls_verify: bool,
}

impl<'a> StartCommand<'a> {
    #[inline]
    pub fn name(&mut self, name: String) -> &mut Self {
        self.name = Some(name);
        self
    }

    pub(crate) fn rm(&mut self) -> &mut Self {
        self.rm = true;
        self
    }

    pub(crate) fn rmi(&mut self) -> &mut Self {
        self.rmi = true;
        self
    }

    pub(crate) fn tls_verify(&mut self, tls_verify: bool) -> &mut Self {
        self.tls_verify = tls_verify;
        self
    }

    pub(crate) fn execute(&self) -> Result<Output, CommandError> {
        let mut cmd = self.podman.mk_podman_command();

        cmd.arg("run");
        cmd.arg(format!("--tls-verify={}", self.tls_verify));
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

        cmd.arg(&self.image_url);

        checked_output(cmd)
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

fn checked_output(mut cmd: Command) -> Result<Output, CommandError> {
    debug!(?cmd, "running command");
    let output = cmd.output()?;

    if !output.status.success() {
        return Err(CommandError {
            err: io::Error::new(io::ErrorKind::Other, "non-zero exit status"),
            stdout: Some(output.stdout),
            stderr: Some(output.stderr),
        });
    }

    Ok(output)
}
