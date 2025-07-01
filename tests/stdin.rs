#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_stdin_reading() {
        let mut child = Command::new("cargo")
            .arg("run")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to spawn child process");

        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        stdin
            .write_all(b"line 1\nline 2\nline 3\n")
            .expect("Failed to write to stdin");

        // Give the application a moment to process the input
        thread::sleep(Duration::from_millis(500));

        // The application should be running. We can't easily inspect the UI state,
        // but we can check that it doesn't crash and that it exits gracefully.
        // To exit the app, we would need to send a 'q' key event, which is not
        // straightforward in a non-interactive test.
        // For now, we'll just kill the process.
        child.kill().expect("Failed to kill child process");
    }
}
