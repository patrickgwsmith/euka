use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::session::AgentAccess;

#[cfg(target_os = "macos")]
const READ_ONLY_PROJECT_PROFILE: &str =
    "(version 1)(allow default)(deny file-write* (subpath (param \"EUKA_ROOT\")))";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentUser {
    Current,
    Staffer,
}

pub fn command(program: &str, user: AgentUser) -> Command {
    match user {
        AgentUser::Current => Command::new(program),
        AgentUser::Staffer => {
            let mut command = Command::new("sudo");
            command.args(["-n", "-H", "-u", "staffer", "--", program]);
            command
        }
    }
}

pub fn workspace_command(
    program: &str,
    user: AgentUser,
    cwd: &Path,
    access: AgentAccess,
) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    if access == AgentAccess::ReadOnly {
        let root = read_only_root(cwd)?;
        let root = root
            .to_str()
            .ok_or("read-only workspace path is not valid UTF-8")?;
        let mut command = command("/usr/bin/sandbox-exec", user);
        command.args([
            "-D",
            &format!("EUKA_ROOT={root}"),
            "-p",
            READ_ONLY_PROJECT_PROFILE,
            program,
        ]);
        return Ok(command);
    }
    let _ = (cwd, access);
    Ok(command(program, user))
}

#[cfg(target_os = "macos")]
fn read_only_root(cwd: &Path) -> Result<std::path::PathBuf, String> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("resolve workspace for read-only agent: {error}"))?;
    Ok(cwd
        .ancestors()
        .find(|directory| directory.join(".git").exists())
        .unwrap_or(&cwd)
        .to_path_buf())
}

pub fn run(
    program: &str,
    args: &[&str],
    task: &str,
    cwd: &Path,
    context: &str,
    user: AgentUser,
    access: AgentAccess,
) -> Result<String, String> {
    let mut child = workspace_command(program, user, cwd, access)?
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    let input = format!("Euka session context:\n{context}\nUser request:\n{task}\n");
    child
        .stdin
        .take()
        .ok_or_else(|| format!("{program} stdin unavailable"))?
        .write_all(input.as_bytes())
        .map_err(|error| format!("{program} stdin: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{program} {}: {}", output.status, stderr.trim()));
    }
    let answer = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if answer.is_empty() {
        return Err(format!("{program} returned no answer"));
    }
    Ok(answer)
}
