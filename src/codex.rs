use crate::agent_cli::{self, AgentUser};
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
    let request = request_for_access(task, access);
    agent_cli::run("codex", &args, &request, cwd, context, user, access)
}

fn request_for_access(task: &str, access: AgentAccess) -> String {
    match access {
        AgentAccess::ReadOnly => format!("{task}\n\n{}", agent_cli::READ_ONLY_REPLY_INSTRUCTION),
        AgentAccess::ReadWrite => task.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::request_for_access;
    use crate::session::AgentAccess;

    #[test]
    fn read_only_request_asks_for_one_line() {
        let request = request_for_access("Summarize this", AgentAccess::ReadOnly);
        assert!(request.starts_with("Summarize this\n\n"));
        assert!(request.contains("one concise plain-text line"));
        assert_eq!(
            request_for_access("Make a change", AgentAccess::ReadWrite),
            "Make a change"
        );
    }
}
