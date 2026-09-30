use crate::agent_cli::{self, AgentUser};
use crate::session::AgentAccess;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

const PROMPT: &str = "Answer the Euka user request supplied on stdin. Use the Euka session context and inspect project files as needed.";

pub fn run(
    task: &str,
    cwd: &Path,
    context: &str,
    answer_chars: usize,
    model: Option<&str>,
    access: AgentAccess,
    user: AgentUser,
) -> Result<String, String> {
    let format_prompt = format!(
        "Answer in one concise plain-text line of at most {answer_chars} characters. Give the finding directly; omit headings and markdown."
    );
    let mut command = agent_cli::workspace_command("claude", user, cwd, access)?;
    let tools = match access {
        AgentAccess::ReadOnly => "Read,Glob,Grep",
        AgentAccess::ReadWrite => "Read,Glob,Grep,Edit,Write,Bash",
    };
    command.args([
        "-p",
        "--restricted",
        "--strict-mcp-config",
        "--tools",
        tools,
        "--allowedTools",
        tools,
        "--disallowedTools",
        "mcp__*",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--no-session-persistence",
        "--output-format",
        "json",
    ]);
    let instruction = match access {
        AgentAccess::ReadOnly => format_prompt,
        AgentAccess::ReadWrite => "Carry out the user's task and report what changed.".to_owned(),
    };
    command.args([
        "--append-system-prompt",
        &format!(
            "{instruction} Do not mention unavailable connectors or integrations unless the user asks about them or they prevent the task."
        ),
    ]);
    if let Some(model) = model {
        command.args(["--model", model]);
    }
    let mut child = command
        .arg(PROMPT)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("claude: {error}"))?;
    let input = format!("Euka session context:\n{context}\nUser request:\n{task}\n");
    child
        .stdin
        .take()
        .ok_or("claude stdin unavailable")?
        .write_all(input.as_bytes())
        .map_err(|error| format!("claude stdin: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("claude: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude {}: {}", output.status, stderr.trim()));
    }
    parse_result(&output.stdout)
}

fn parse_result(output: &[u8]) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(output)
        .map_err(|error| format!("claude returned invalid JSON: {error}"))?;
    let result = value["result"]
        .as_str()
        .ok_or("claude returned no result")?;
    if value["is_error"].as_bool() == Some(true) {
        return Err(result.to_owned());
    }
    Ok(result.to_owned())
}

#[cfg(test)]
mod tests {
    use super::parse_result;

    #[test]
    fn result_errors_are_not_shown_as_answers() {
        assert_eq!(
            parse_result(br#"{"type":"result","is_error":false,"result":"found it"}"#).unwrap(),
            "found it"
        );
        assert_eq!(
            parse_result(br#"{"type":"result","is_error":true,"result":"denied"}"#).unwrap_err(),
            "denied"
        );
    }
}
