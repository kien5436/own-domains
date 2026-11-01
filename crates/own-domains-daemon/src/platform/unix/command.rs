//! Host command execution shared by Unix adapters.
//!
//! Bounded output, timeouts, and no shell: every argument is passed
//! explicitly to the spawned program.

use std::{path::PathBuf, time::Duration};

pub(crate) const MAX_COMMAND_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        use std::{
            process::{Command, Stdio},
            thread,
            time::Instant,
        };

        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("unable to run {}: {error}", command.program.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "unable to capture command stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "unable to capture command stderr".to_string())?;
        let stdout = thread::spawn(move || read_bounded_command_output(stdout));
        let stderr = thread::spawn(move || read_bounded_command_output(stderr));
        let deadline = Instant::now() + command.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} did not finish within {:?}",
                        command.program.display(),
                        command.timeout
                    ));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "unable to wait for {}: {error}",
                        command.program.display()
                    ));
                }
            }
        };
        let stdout = stdout
            .join()
            .map_err(|_| "command stdout reader panicked".to_string())??;
        let stderr = stderr
            .join()
            .map_err(|_| "command stderr reader panicked".to_string())??;
        Ok(HostCommandOutput {
            status: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}

fn read_bounded_command_output(mut reader: impl std::io::Read) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_COMMAND_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("unable to read command output: {error}"))?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(format!(
            "command output exceeds the {MAX_COMMAND_BYTES}-byte limit"
        ));
    }
    Ok(bytes)
}
