use crate::agent_cli::AgentUser;
use crate::session::AgentAccess;
use std::path::Path;

pub fn run(
    task: &str,
    cwd: &Path,
    context: &str,
    model: Option<&str>,
    access: AgentAccess,
    user: AgentUser,
) -> Result<String, String> {
    let mut args = vec!["--ask-for-approval", "never", "exec"];
    if access == AgentAccess::ReadOnly {
        args.push("--ignore-user-config");
    }
    args.extend([
        "--sandbox",
        match access {
            AgentAccess::ReadOnly => "read-only",
            AgentAccess::ReadWrite => "workspace-write",
        },
        "--ephemeral",
        "--skip-git-repo-check",
        "--color",
        "never",
    ]);
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    args.push("-");
    super::agent_cli::run("codex", &args, task, cwd, context, user, access)
}
