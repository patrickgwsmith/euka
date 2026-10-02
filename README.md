# Euka

Euka is an experimental shell for humans and coding agents. You type shell commands and agent requests at the same prompt.

Euka is short for eukaryotic, cells evolved from archaea and bacterial partners such as the mitochondria, coming together to form a more complex cell. Euka uses the same idea: it brings a shell and coding agents into one session. The shell runs your commands and various agents handle coding requests amongst each other.

## Install

With Rust and Cargo installed, install the package from crates.io and start Euka:

```sh
cargo install euka-shell
euka
```

The package is named `euka-shell`; the executable is `euka`.

## Why use Euka

A coding agent can usually change all the files in your project. Euka gives you more control:

- Each agent request has a suffix. The suffix sets the access level of the request.
  - `?` gives read-only access. The agent can read files, but it cannot change them.
  - `!` gives read-write access. The agent can change files and run commands.
- On macOS, a system sandbox prevents file writes during `?` requests, except to temporary directories and the agent's own state. This protection stays on if the agent's own restrictions fail.
- You can run agents and shell commands as a separate Unix user, `staffer`. This user has less access than your own account.
- All agents share one session. An agent can see your notes, recent command input (and exit statuses but not output), watched command output, and answers from other agents.

Watch a command with `+ bash? git status`, or watch an Atom or RSS feed with `+ FEED URL`. Euka shares the results with agents; enter `+` to refresh the watches. See [Watch a command or URL](#watch-a-command-or-url) for examples.

Euka does not give full protection. Read [Limits of the protection](#limits-of-the-protection) before you use it.

## Requirements

- macOS or Linux.
- Rust and Cargo, to install or build Euka.
- One or more agent CLIs, installed and authenticated: Claude Code, Codex, or Pi.
- Optional, on macOS: Apple's `fm` CLI, with its license accepted.

## Build and run

Run `make` to rebuild both debug and release binaries. Use `make debug` or `make release` for just one, and `make test` to run the tests. The binaries are at `target/debug/euka` and `target/release/euka`.

To build and start Euka in one step, run `cargo run`. Run `cargo run -- --help` to show help and exit. At the Euka prompt, enter `?` to show a cheatsheet of prefixes and built-in commands. On macOS, the cheatsheet also shows the command that creates the `staffer` account.

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
| `claude`, `fable`, `opus`, `sonnet`, `haiku` | Claude Code |
| `codex`, `sol`, `luna`, `terra` | Codex |
| `pi` | Pi |
| `fm` | Apple Foundation Models (macOS only) |
| `jsc` | JavaScriptCore shell (macOS only) |

Each request runs in the background. You can continue to type commands while the agent works.

For a short read-only request, use `🎵 task` or `🎶 task` for `opus? task`, `☀️ task` for `sol? task`, `🌙 task` for `luna? task`, `🥧 task` for `pi? task`, or `🍎 task` for `fm? task`.

`Opus? task` also works as `opus? task`.

You can paste a multiline request, including its `opus?` prefix, into the interactive prompt. Euka keeps the pasted line breaks in one request; press Enter after pasting to send it.

Enter an agent name with no request, for example `opus?` or `codex!`, to open that agent's own interactive CLI in the terminal. With `?` the session is read-only, using the same read-only tools and macOS sandbox as a `?` request. With `!` it can change files, using the same tools and sandbox settings as a `!` request. Bare `jsc?` or `jsc!` opens the jsc REPL, and bare `fm?` or `fm!` enters fm mode. When you exit the agent, you return to the Euka prompt.

`jsc` is not a model: Euka runs the request as JavaScript in the macOS JavaScriptCore shell and replies with what the script prints. Use `print()` for output, for example `jsc? print(6 * 7)`.

### How Euka applies read-only access

For a `?` request, Euka uses these restrictions:

- **Codex:** Euka uses the Codex `read-only` sandbox. Euka ignores your Codex user configuration, so an automatic approval setting cannot cancel read-only mode.
- **Claude Code:** Euka uses restricted mode and an explicit list of tools.
- **Pi:** Euka uses an explicit list of tools.
- **fm:** Euka controls the tools. The agent can list directories and read files in the current directory. It cannot write files or run commands.
- **macOS system sandbox:** For Claude Code, Codex, Pi, and jsc, a macOS sandbox denies all file writes except to `/dev`, the temporary directories (`/tmp` and `/var/folders`), and the agent user's `~/.claude`, `~/.claude.json*`, `~/.codex`, `~/.pi`, `~/.cache`, and `~/Library/Caches`. This protects your project, its Git index, other repositories, and the rest of your home directory.

For a `!` request, Codex uses its `workspace-write` sandbox. Other agents can write files and run commands.

### Run an agent as the `staffer` user

Put `@staffer` before a request to run the agent as the `staffer` Unix user:

```text
@staffer opus? Is there a README.md yet?
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

Each request gets a label, for example `codex?#3` for read-only access or `opus!#4` for read-write access. Numbers are unique across all agents.

- When the request starts, Euka shows `codex?#3: …`.
- When the answer is ready, Euka shows `codex?#3: answer`.
- To show the latest completed agent reply, enter `#`. To show a specific full answer, enter `#3` or `codex?#3`. Older labels such as `codex#3` also work.
- Claude and Codex read-only agents are asked for a concise one-line answer. Euka displays their replies without clipping; long lines wrap on screen.
- To give an answer to a different agent, refer to it in the request. For example: `claude? do you agree with #3?`
- If a request refers to an unfinished answer, Euka queues it in the background, shows `waiting for #3`, and starts it when the answer arrives. A request with several references waits for all of them. Euka builds the queued request's context after those answers enter the session log and includes each referenced answer once. If a referenced request fails, the queued request reports the failure.
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

To send many read-only `fm` requests, enter `fm?` or `fm!` with no task. The prompt changes to `fm?>`. Each line that you type is then an `fm?` request. To go back to the shell prompt, enter `.` or `exit`, or press Ctrl-C. Agent requests continue to run after you go back.

Euka examines tool access for each `fm` step. Euka skips repeated tool calls. When the step limit is reached, Euka tells the agent to give a final answer.

For a summary of earlier session results, such as `fm? summarize the ops/s`, Euka sends the shared session context directly to `fm` instead of starting a file search. Refer to a specific reply with `#n` when you want that reply alone summarized.

## Shell commands

### What agents see from commands

For an ordinary command such as `git status`, Euka shows you the output in the terminal, but gives agents only the command text and exit status. It does not add that command's stdout or stderr to the shared session.

To share a command's output with agents, register a watch such as `+ bash? git status`. Euka includes watch output in agent context. Shell watches run as `staffer`, and you refresh them by entering `+` or `+-`. Agents can also run commands themselves when their access allows it, so this controls what Euka shares automatically rather than what an agent can ever learn.

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

### Watch a command or URL

Put `+` before `bash?`, `zsh?`, `HEAD`, or `FEED` to watch a command or URL:

```text
+ bash? git status
+ HEAD https://github.com/patrickgwsmith/euka
+ FEED https://github.com/patrickgwsmith/euka/commits/main.atom
+ FEED https://static.crates.io/rss/updates.xml
+ https://github.com/patrickgwsmith/euka/commits/main.atom
```

- Euka runs each watch once when registered and again only when you enter `+` or `+-`. Shell watches run as `staffer`; HEAD and feed watches use Euka's HTTP client.
- `+ URL` accepts Atom and RSS feeds when their responses have `Content-Type: application/atom+xml` or `application/rss+xml`. Use `+ FEED URL` to parse a feed served with a generic XML content type, such as the crates.io RSS feed. Feed entries show their title and link; `+` shows new entries and `+-` also shows entries that disappeared from the feed.
- Agents can see watch output in the shared session.
- Shell watches need passwordless `sudo` for `staffer`. Errors show at the prompt.
- You can watch more than one command or directory at the same time.

Enter `+` alone to run each watch again and show only added lines. Enter `+-` to show both added and removed lines. Unchanged lines are omitted. Each check compares with the previous `+` or `+-` check, or with the first result if you have not checked yet.

Every watch, HTTP request, and agent request uses the same number sequence. A watch's first successful result is revision 1; Euka adds another revision whenever its output changes. For example, `#3` shows watch #3's latest successful result, and `#3.2` shows its second revision. Agents can use the same references: `opus? What changed between #3.1 and #3.2?` A request for a watch still loading waits for its first result. Repeating the same `+` command keeps the existing watch number; enter `+` to refresh it. Watch revisions last until `reset` or exit.

Numbered HTTP results, such as `[head #2]`, can also be referenced in an agent request with `#2`.

### Example: build a Base64 crate

In a fresh Euka session, write down the goal, create the crate, and watch recent crates.io releases. The feed is shared context; the crate's feature plan does not depend on a particular release appearing in it. Ask Opus to research the feature set, Pi to implement it, and Luna to review the code. This transcript shows illustrative, shortened output; enter each request after the preceding reply arrives:

```text
# I want to make a base64 encoder in Rust
cargo new --lib mini-base64
cd mini-base64
+ FEED https://static.crates.io/rss/updates.xml
[watch #1 loading] + FEED https://static.crates.io/rss/updates.xml
[watch #1.1 + FEED https://static.crates.io/rss/updates.xml] New crate version published: ...
opus? Research the features a small Base64 Rust crate should support. Suggest a public API and test cases for standard and URL-safe encoding.
opus?#2: Support both alphabets, specify padding behavior, and test RFC 4648 vectors plus empty and non-ASCII bytes.
pi! Implement the API and tests proposed in #2, with documentation for standard and URL-safe encoding.
pi!#3: Added standard and URL-safe encoders, padding options, documentation, and test vectors.
luna? Review #3, src/lib.rs, and the tests for correctness and API clarity before publishing.
luna?#4: The encoders and tests cover the key cases; check the package metadata before publishing.
```

The first `#` line is a note shared with all agents. The feed is `#1`, Opus's research is `#2`, and Pi's implementation is `#3`. You can send Luna's review request while Pi is still working; referring to `#3` queues it until Pi's result arrives. After you review the code, address Luna's feedback, choose an available crate name, and fill in the crate's metadata, you run the publication commands yourself:

```text
cargo test
cargo publish --dry-run
cargo publish
```

### Notes, todos, and URLs

Euka keeps these items in the session, and agents can see them:

- Comments that start with `#`.
- Todos that start with `- [ ]`.
- Web pages. Enter an HTTP or HTTPS URL to load the page in the background. Euka keeps up to 4 MiB of each page in memory. Agents get a part of the page.

Enter `-` alone to show the current TODO list.

Euka does not follow redirects. It shows the new URL. To load that page, enter the new URL.

Enter `HEAD https://...` to show the response headers and the time to receive them. Enter `+ HEAD https://...` to register a HEAD watch, then enter `+` when you want to refresh it and compare results. Euka does not (yet) show separate DNS, TCP, and TLS times.

### Reset the session

Enter `reset` to clear:

- The session.
- The input history.
- The watched commands.
- The request numbers. The next request is `#1`.

`reset` does not change the current directory or the environment. Requests that are in progress can continue to run, but Euka discards their results. To reset the terminal, enter `bash! reset`.

## Prompt and editing

The prompt shows a short form of the current directory. It shows the first character of each parent directory and the full name of the current directory. It shows your home directory as `~`. For example, Euka shows `/Users/pgwsmith/Collected/euka` as `~/C/euka>`.

In an interactive terminal, Euka sets the terminal title to the same shortened directory path shown in the prompt and updates it after `cd` or a child command changes the title.

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

- The macOS sandbox still lets a `?` agent write to temporary directories and to the agent CLIs' own state and cache directories in the agent user's home directory.
- On Linux, `?` requests use only the restrictions of each agent CLI. Euka does not add a system sandbox.
- `!` requests can change files and run commands.
- `bash?`, `zsh?`, and `@staffer` have all the access of the `staffer` user.

## Other limits

- Euka does not support Bash control syntax, globbing, or persistence. Use `bash!` or `zsh!` for these.
- History and the session are in memory only. Euka discards them when you exit.
- Euka reserves `?` and `!` without an agent name for a future default model.
- Euka recognizes all `name?` and `name!` forms as agent requests. If the agent is not available, Euka shows an error and does not run a shell command. Names can contain letters, digits, underscores, hyphens, and periods. The `?` or `!` must be at the end of the first word.
- Euka reserves other syntax for future use.

## TODO

- [ ] Show live status for background Codex requests by reading `codex exec --json` events as they arrive, similar to Claude Code's `stream-json` output. Keep the final answer as the numbered result.
