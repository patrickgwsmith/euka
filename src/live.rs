use std::io::{self, Read};
use std::path::Path;
use std::process::Stdio;
use std::thread;

const MAX_LIVE_LINES: usize = 200;
const MAX_LIVE_BYTES: usize = 64 * 1024;

pub fn run(program: &str, script: &str, cwd: &Path) -> Result<String, String> {
    let mut child = super::shell::reader_shell_command(program, cwd, true)
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}?: {error}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || read_capped(stdout));
    let stderr_reader = thread::spawn(move || read_capped(stderr));
    let status = child.wait().map_err(|error| error.to_string())?;
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| "stdout reader stopped".to_owned())?
        .map_err(|error| error.to_string())?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| "stderr reader stopped".to_owned())?
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let mut text = stdout.trim_end().to_owned();
    if !stderr.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("stderr: ");
        text.push_str(stderr.trim_end());
    }
    if stdout_truncated || stderr_truncated {
        text.push_str("\n... output truncated");
    }
    if !status.success() {
        return Err(format!("exit {status}: {}", limit(&text)));
    }
    if text.is_empty() {
        text.push_str("(no output)");
    }
    Ok(limit(&text))
}

fn read_capped(mut pipe: impl Read) -> io::Result<(Vec<u8>, bool)> {
    let mut result = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let keep = MAX_LIVE_BYTES.saturating_sub(result.len()).min(count);
        result.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    Ok((result, truncated))
}

fn limit(text: &str) -> String {
    let mut result = String::new();
    for (lines, line) in text.lines().enumerate() {
        if lines == MAX_LIVE_LINES {
            result.push_str("\n... output truncated");
            break;
        }
        let separator = usize::from(lines > 0);
        if result.len() + separator + line.len() > MAX_LIVE_BYTES {
            if lines > 0 {
                result.push('\n');
            }
            let mut end = MAX_LIVE_BYTES.saturating_sub(result.len()).min(line.len());
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            result.push_str(&line[..end]);
            result.push_str("\n... output truncated");
            break;
        }
        if lines > 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{limit, read_capped, MAX_LIVE_BYTES};

    #[test]
    fn large_live_output_is_capped_and_drained() {
        let data = vec![b'x'; MAX_LIVE_BYTES + 100];
        let (captured, truncated) = read_capped(data.as_slice()).unwrap();
        assert_eq!(captured.len(), MAX_LIVE_BYTES);
        assert!(truncated);
        let display = limit(&format!(
            "{}\n... output truncated",
            String::from_utf8(captured).unwrap()
        ));
        assert!(display.starts_with("xxxx"));
        assert!(display.ends_with("... output truncated"));
    }
}
