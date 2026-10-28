use std::{
    env,
    io::{self, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

fn fixture_name() -> &'static str {
    Path::new(env!("CARGO_BIN_EXE_shell_fixture"))
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fixture has an ASCII executable name")
}

fn execute(input: &[u8]) -> io::Result<Output> {
    let fixture_dir = Path::new(env!("CARGO_BIN_EXE_shell_fixture"))
        .parent()
        .expect("fixture has a parent directory");
    let mut search_paths = vec![fixture_dir.to_path_buf()];
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path = env::join_paths(search_paths).map_err(io::Error::other)?;

    let mut shell = Command::new(env!("CARGO_BIN_EXE_blazing_shell"))
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Err(error) = shell
        .stdin
        .take()
        .expect("piped stdin exists")
        .write_all(input)
    {
        let kill = shell.kill();
        let wait = shell.wait();
        return Err(io::Error::new(
            error.kind(),
            format!("input write failed: {error}; cleanup kill: {kill:?}, wait: {wait:?}"),
        ));
    }
    shell.wait_with_output()
}

#[test]
fn failed_input_write_reports_cleanup_outcome() {
    let mut input = b"exit\n".to_vec();
    input.extend(vec![b'\n'; 262_144]);
    let error = execute(&input).expect_err("shell exits before accepting the full input");
    let message = error.to_string();
    assert!(message.contains("input write failed"), "{message}");
    assert!(message.contains("cleanup kill:"), "{message}");
    assert!(message.contains("wait:"), "{message}");
}

#[test]
fn single_command_waits_and_exit_stops_shell() -> io::Result<()> {
    let input = format!(
        "{} emit hello\nexit\n{} emit should-not-run\n",
        fixture_name(),
        fixture_name()
    );
    let output = execute(input.as_bytes())?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("ASCII fixture output");
    assert!(stdout.contains("hello\n"), "{stdout}");
    assert!(!stdout.contains("should-not-run"));
    assert_eq!(stdout.matches("blazing_shell> ").count(), 2);
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn bad_commands_nonzero_status_and_empty_input_do_not_terminate_shell() -> io::Result<()> {
    let input = format!(
        "\n__blazing_shell_missing_executable_449aa__\n{} fail\n{} emit recovered\nexit\n",
        fixture_name(),
        fixture_name()
    );
    let output = execute(input.as_bytes())?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("ASCII fixture output");
    let stderr = String::from_utf8(output.stderr).expect("ASCII diagnostics");
    assert!(stdout.contains("recovered\n"));
    assert_eq!(stdout.matches("blazing_shell> ").count(), 5);
    assert!(stderr.contains("Ошибка запуска"), "{stderr}");
    assert!(stderr.contains("exit status:"), "{stderr}");
    Ok(())
}

#[test]
fn pipeline_transfers_bytes_and_reports_status() -> io::Result<()> {
    let fixture = fixture_name();
    let input = format!("{fixture} emit hello|{fixture} upper\nexit\n");
    let output = execute(input.as_bytes())?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("HELLO\n"));
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn pipeline_streams_more_than_pipe_capacity() -> io::Result<()> {
    let fixture = fixture_name();
    let input = format!("{fixture} large | {fixture} count\nexit\n");
    let output = execute(input.as_bytes())?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("262144\n"));
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn failed_right_hand_spawn_and_invalid_utf8_allow_next_command() -> io::Result<()> {
    let fixture = fixture_name();
    let mut input =
        format!("{fixture} large | __blazing_shell_missing_executable_449aa__\n").into_bytes();
    input.extend_from_slice(b"\xff\n");
    input.extend_from_slice(format!("{fixture} emit recovered\nexit\n").as_bytes());
    let output = execute(&input)?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("recovered\n"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Ошибка запуска"), "{stderr}");
    assert!(stderr.contains("UTF-8"), "{stderr}");
    Ok(())
}

#[test]
fn malformed_pipe_and_end_of_file_exit_cleanly() -> io::Result<()> {
    let input = format!(
        "| {} upper\n{} emit after-error\n",
        fixture_name(),
        fixture_name()
    );
    let output = execute(input.as_bytes())?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("after-error\n"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Ошибка команды"));
    Ok(())
}
