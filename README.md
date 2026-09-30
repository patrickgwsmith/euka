# Euka

Euka is an experimental shell for humans and coding agents. You type shell commands and agent requests at the same prompt.

Euka is short for eukaryotic, cells evolved from archaea and bacterial partners such as the mitochondria, coming together to form a more complex cell. Euka uses the same idea: it brings a shell and coding agents into one session. The shell runs your commands and various agents handle coding requests amongst each other.

## Why use Euka

A coding agent can usually change all the files in your project. Euka gives you more control:

- Each agent request has a suffix. The suffix sets the access level of the request.
  - `?` gives read-only access. The agent can read files, but it cannot change them.
  - `!` gives read-write access. The agent can change files and run commands.
- On macOS, a system sandbox prevents writes to your project during `?` requests. This protection stays on if the agent's own restrictions fail.
- You can run agents and shell commands as a separate Unix user, `staffer`. This user has less access than your own account.
- All agents share one session. An agent can see your notes, your commands, and the answers from other agents.

Euka does not give full protection. Read [Limits of the protection](#limits-of-the-protection) before you use it.

## Requirements

- macOS or Linux.
- Rust and Cargo, to build Euka.
- One or more agent CLIs, installed and authenticated: Claude Code, Codex, or Pi.
- Optional, on macOS: Apple's `fm` CLI, with its license accepted.

## Build and run

1. To build and start Euka, run `cargo run`.
2. To show help and exit, run `cargo run -- --help`.
3. At the Euka prompt, enter `?` to show a cheatsheet of prefixes and built-in commands. On macOS, the cheatsheet also shows the command that creates the `staffer` account.

## Agent requests

### Send a request

Type the agent name, the suffix, and the task:

```text
claude? explain how src/session.rs classifies input
codex! add a test for the input parser
```

Euka supports these agent names:

| Name | Agent |
| --- | --- |
| `claude`, `opus`, `sonnet` | Claude Code |
| `codex`, `sol`, `luna`, `terra` | Codex |
| `pi` | Pi |
| `fm` | Apple Foundation Models (macOS only) |

Each request runs in the background. You can continue to type commands while the agent works.

### How Euka applies read-only access

For a `?` request, Euka uses these restrictions:

- **Codex:** Euka uses the Codex `read-only` sandbox. Euka ignores your Codex user configuration, so an automatic approval setting cannot cancel read-only mode.
- **Claude Code:** Euka uses restricted mode and an explicit list of tools.
- **Pi:** Euka uses an explicit list of tools.
- **fm:** Euka controls the tools. The agent can list directories and read files in the current directory. It cannot write files or run commands.
- **macOS system sandbox:** For Claude Code, Codex, and Pi, a macOS sandbox prevents writes to the current Git working tree. Outside a Git repository, it prevents writes to the current directory. This sandbox also protects the Git index.

For a `!` request, Codex uses its `workspace-write` sandbox. Other agents can write files and run commands.

### Run an agent as the `staffer` user

Put `@staffer` before a request to run the agent as the `staffer` Unix user:

```text
@staffer opus? Is there a LICENSE?
```

Before you use `@staffer`, do these steps:

1. Create the `staffer` account. On macOS, enter `?` to see the command.
2. Configure passwordless `sudo` for `staffer`. Euka uses noninteractive `sudo`.
3. Give `staffer` access to the agent CLI.
4. Authenticate the agent CLI as `staffer`.
5. Give `staffer` permission to read the project.

You cannot use `@staffer` with `fm`, because the `fm` tools run inside Euka.

### Ask more than one agent

Use a slash between agent names to send the same read-only question to each agent:

```text
opus/luna? what is a modern alternative to tree?
```

Each agent runs in the background and gets its own reply label.

### Read and refer to answers

Each request gets a label, for example `codex#3`. Numbers are unique across all agents.

- When the request starts, Euka shows `codex#3: …`.
- When the answer is ready, Euka shows `codex#3: answer`.
- To show the full answer, enter `#3` or `codex#3`.
- To give an answer to a different agent, refer to it in the request. For example: `claude? do you agree with #3?`
- If a request refers to an unfinished answer, Euka queues it in the background, shows `waiting for #3`, and starts it when the answer arrives. A request with several references waits for all of them. If a referenced request fails, the queued request reports the failure.
- Entering `#3` by itself while it is still running reports that it is still in progress.

Labels stay available until you exit Euka or enter `reset`.

### Put an agent answer in a command

Use `name?(task)` as one argument of a shell command. Euka waits for the read-only answer, then gives the answer text to the command as one argument:

```text
git commit -m luna?(Suggest a commit message)
```

Obey these rules:

- Do not put `(` in the task. The first `)` ends the task.
- Do not put text immediately after the `)`.
- If the `)` is missing, Euka shows an error.

### Use fm mode

On macOS, `fm?` and `fm!` use Apple Foundation Models:

```text
fm? explain how src/session.rs classifies input
fm! create a file named note.txt containing hello
```

To send many read-only `fm` requests, enter `fm?` with no task. The prompt changes to `fm?>`. Each line that you type is then an `fm?` request. To go back to the shell prompt, enter `.` or `exit`, or press Ctrl-C. Agent requests continue to run after you go back.

Euka examines tool access for each `fm` step. Euka skips repeated tool calls. When the step limit is reached, Euka tells the agent to give a final answer.

## Shell commands

### Commands that run as you

Euka runs these commands directly, without an LLM:

- Commands from `PATH`, with arguments, quotes, and backslash escapes.
- `$NAME` and `$?` expansion.
- Pipelines and the redirections `<`, `>`, `>>`, `2>`, and `2>>`.
- Built-in commands: `cd`, `export NAME=value`, `unset NAME`, `NAME=value`, and `exit`.

If a command contains `$(...)`, Euka runs the full command through Bash or Zsh. Euka uses `$SHELL` if it is Bash or Zsh. If not, Euka uses Zsh on macOS and Bash on Linux. Built-in commands such as `cd` in this shell do not change the directory of Euka.

To run a command through a specified shell, use `bash!` or `zsh!`:

```text
bash! for f in *.rs; do wc -l "$f"; done
```

Euka does not change the script and does not use an LLM. Changes to the directory or environment of the child shell do not go back to Euka.

### Commands that run as `staffer`

`bash? command` and `zsh? command` run the command as the `staffer` user through `sudo`.

- In interactive input, `sudo` can ask for your password.
- In noninteractive input, Euka uses `sudo -n`. If `sudo` needs a password, the command fails immediately.
- If your Euka user owns the Git repository, Euka lets `staffer` read it with Git. Euka sets `safe.directory` for that one command only.

**Warning:** `staffer` can write to all paths that `staffer` has write access to, for example group-writable paths. `bash?` and `zsh?` are not a read-only sandbox.

### Watch a command

Put `+` before `bash?` or `zsh?` to watch a command:

```text
+ bash? git status
```

- Euka runs the command as `staffer` every two seconds.
- Agents can see the output in the shared session.
- You must configure passwordless `sudo` for `staffer` first. Errors show at the prompt.
- You can watch more than one command or directory at the same time.

Enter `+` alone to run each watched command again. Euka then shows the lines that changed since the last `+`, or since the first result.

### Notes, todos, and URLs

Euka keeps these items in the session, and agents can see them:

- Comments that start with `#`.
- Todos that start with `- [ ]`.
- Web pages. Enter an HTTP or HTTPS URL to load the page in the background. Euka keeps up to 4 MiB of each page in memory. Agents get a part of the page.

Euka does not follow redirects. It shows the new URL. To load that page, enter the new URL.

Enter `HEAD https://...` to show the response headers and the time to receive them. Euka does not (yet) show separate DNS, TCP, and TLS times.

### Reset the session

Enter `reset` to clear:

- The session.
- The input history.
- The watched commands.
- The request numbers. The next request is `#1`.

`reset` does not change the current directory or the environment. Requests that are in progress can continue to run, but Euka discards their results. To reset the terminal, enter `bash! reset`.

## Prompt and editing

The prompt shows a short form of the current directory. It shows the first character of each parent directory and the full name of the current directory. It shows your home directory as `~`. For example, Euka shows `/Users/pgwsmith/Collected/euka` as `~/C/euka>`.

To show the full path for three seconds, press Down at an empty prompt.

Colors help you see the access level before you press Enter:

| Color | Input |
| --- | --- |
| Cyan | The prompt |
| Magenta | Agent requests, for example `opus/luna?` and `codex!`, and the `fm?` mode label |
| Blue | Commands as `staffer`: `bash?`, `zsh?`, and `@staffer` |
| Red | Commands as you through a shell: `bash!` and `zsh!` |

Watched commands, for example `+ bash?`, use the same colors. Agent answers color the reply label, Markdown bold text, bullet marks, and inline code. To remove the colors, set `NO_COLOR=1`.

Editing keys:

| Keys | Action |
| --- | --- |
| Tab | Complete a command, file, or directory |
| Ctrl-A, Ctrl-E | Move to the start or end of the line |
| Ctrl-B, Ctrl-F | Move back or forward one character |
| Option-B, Option-F | Move back or forward one word |
| Ctrl-W, Option-D | Delete a word |
| Ctrl-U, Ctrl-K | Delete to the start or end of the line |
| Ctrl-Y | Put back the text that you deleted last |
| Ctrl-P, Ctrl-N | Show the previous or next history item |
| Ctrl-C | Cancel the current line |
| Ctrl-D | Exit Euka (at an empty prompt) |

Tab completion accepts relative paths, absolute paths, and paths that start with `~/`. It adds escapes to spaces in names. Euka also shows suggestions from your history as you type.

## Limits of the protection

- The macOS sandbox prevents writes only in the current Git working tree, or the current directory. A `?` agent can write to other locations.
- On Linux, `?` requests use only the restrictions of each agent CLI. Euka does not add a system sandbox.
- `!` requests can change files and run commands.
- `bash?`, `zsh?`, and `@staffer` have all the access of the `staffer` user.

## Other limits

- Euka does not support Bash control syntax, globbing, or persistence. Use `bash!` or `zsh!` for these.
- History and the session are in memory only. Euka discards them when you exit.
- Euka does not keep command output in the session. Agents see only command names and exit statuses. Watched commands are an exception.
- Euka reserves `?` and `!` without an agent name for a future default model.
- Euka recognizes all `name?` and `name!` forms as agent requests. If the agent is not available, Euka shows an error and does not run a shell command. Names can contain letters, digits, underscores, hyphens, and periods. The `?` or `!` must be at the end of the first word.
- Euka reserves other syntax for future use.
