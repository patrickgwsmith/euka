use crate::agent_cli::{self, AgentUser};
use crate::session::AgentAccess;
use std::path::Path;
use std::process::Command;

/// The JavaScriptCore shell bundled with macOS, which is not on PATH.
const JSC: &str =
    "/System/Library/Frameworks/JavaScriptCore.framework/Versions/Current/Helpers/jsc";

/// Evaluates `task` as a JavaScript program and returns what it prints.
/// `-e` runs the script without the REPL that stdin input would start, so
/// expression results are not echoed. A read-only request runs inside the
/// same write-denying sandbox as the other agents.
pub fn run(task: &str, cwd: &Path, access: AgentAccess, user: AgentUser) -> Result<String, String> {
    let mut command = agent_cli::workspace_command(JSC, user, access)?;
    command.args(["-e", task]).current_dir(cwd);
    let output = agent_cli::run_with_input(command, "jsc", String::new())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        // jsc reports uncaught exceptions on stdout.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = [stdout.as_str(), stderr.trim()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("jsc {}: {detail}", output.status));
    }
    if stdout.is_empty() {
        return Err("jsc printed nothing; use print() for output".to_owned());
    }
    Ok(stdout)
}

/// The jsc REPL, inside the read-only sandbox for `jsc?`.
pub fn interactive(cwd: &Path, access: AgentAccess) -> Result<Command, String> {
    let mut command = agent_cli::workspace_command(JSC, AgentUser::Current, access)?;
    command.current_dir(cwd);
    Ok(command)
}
