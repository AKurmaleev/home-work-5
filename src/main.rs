#![forbid(unsafe_code)]

use std::{
    io::{self, BufRead, Write},
    process::{Child, Command, ExitStatus, Stdio},
};

const PROMPT: &str = "blazing_shell> ";

#[derive(Debug, PartialEq, Eq)]
struct CommandSpec<'a> {
    program: &'a str,
    args: Vec<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
enum Input<'a> {
    Empty,
    Exit,
    Command(CommandSpec<'a>),
    Pipe(CommandSpec<'a>, CommandSpec<'a>),
}

fn command_spec(text: &str) -> Option<CommandSpec<'_>> {
    let mut words = text.split_whitespace();
    Some(CommandSpec {
        program: words.next()?,
        args: words.collect(),
    })
}

fn parse_line(line: &str) -> Result<Input<'_>, &'static str> {
    if line.trim().is_empty() {
        return Ok(Input::Empty);
    }
    if line.trim() == "exit" {
        return Ok(Input::Exit);
    }

    let mut parts = line.split('|');
    let left = parts.next().expect("split always produces a first part");
    let Some(right) = parts.next() else {
        return Ok(Input::Command(
            command_spec(left).expect("nonempty input has a command"),
        ));
    };
    if parts.next().is_some() {
        return Err("поддерживается не более одного pipe");
    }
    let left = command_spec(left).ok_or("отсутствует команда слева от pipe")?;
    let right = command_spec(right).ok_or("отсутствует команда справа от pipe")?;
    Ok(Input::Pipe(left, right))
}

fn report_status(errors: &mut impl Write, program: &str, status: ExitStatus) -> io::Result<()> {
    if !status.success() {
        writeln!(errors, "'{program}': exit status: {status}")?;
    }
    Ok(())
}

fn run_command(spec: CommandSpec<'_>, errors: &mut impl Write) -> io::Result<()> {
    match Command::new(spec.program).args(spec.args).status() {
        Ok(status) => report_status(errors, spec.program, status),
        Err(error) => writeln!(errors, "Ошибка запуска '{}': {error}", spec.program),
    }
}

trait ChildCleanup {
    fn kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<()>;
}

impl ChildCleanup for Child {
    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }

    fn wait(&mut self) -> io::Result<()> {
        Child::wait(self).map(|_| ())
    }
}

fn stop_and_wait(child: &mut impl ChildCleanup) -> io::Result<()> {
    let kill_result = child.kill();
    // Reap the child even when terminating it fails.
    let wait_result = child.wait();
    combine_results(kill_result, wait_result)
}

fn combine_results(primary: io::Result<()>, secondary: io::Result<()>) -> io::Result<()> {
    match (primary, secondary) {
        (Err(primary), Err(secondary)) => Err(io::Error::new(
            primary.kind(),
            format!("{primary}; additional cleanup failure: {secondary}"),
        )),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn report_failure(
    errors: &mut impl Write,
    message: &str,
    cleanup: io::Result<()>,
) -> io::Result<()> {
    let reporting = writeln!(errors, "{message}").map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{message}; diagnostic write failed: {error}"),
        )
    });
    combine_results(reporting, cleanup)
}

fn run_pipe(
    left: CommandSpec<'_>,
    right: CommandSpec<'_>,
    errors: &mut impl Write,
) -> io::Result<()> {
    let mut producer = match Command::new(left.program)
        .args(left.args)
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return writeln!(errors, "Ошибка запуска '{}': {error}", left.program),
    };

    let Some(reader) = producer.stdout.take() else {
        let cleanup = stop_and_wait(&mut producer);
        return combine_results(
            Err(io::Error::other(
                "не удалось получить stdout первого процесса",
            )),
            cleanup,
        );
    };
    let mut consumer = match Command::new(right.program)
        .args(right.args)
        .stdin(Stdio::from(reader))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let cleanup_result = stop_and_wait(&mut producer);
            return report_failure(
                errors,
                &format!("Ошибка запуска '{}': {error}", right.program),
                cleanup_result,
            );
        }
    };

    // The parent owns no read end: EOF and broken-pipe reach the right processes.
    let right_status = match consumer.wait() {
        Ok(status) => status,
        Err(error) => {
            let consumer_cleanup = stop_and_wait(&mut consumer);
            let producer_cleanup = stop_and_wait(&mut producer);
            return report_failure(
                errors,
                &format!("Ошибка ожидания '{}': {error}", right.program),
                combine_results(consumer_cleanup, producer_cleanup),
            );
        }
    };
    let left_status = match producer.wait() {
        Ok(status) => Some(status),
        Err(error) => {
            let cleanup_result = stop_and_wait(&mut producer);
            report_failure(
                errors,
                &format!("Ошибка ожидания '{}': {error}", left.program),
                cleanup_result,
            )?;
            None
        }
    };
    if let Some(status) = left_status {
        report_status(errors, left.program, status)?;
    }
    report_status(errors, right.program, right_status)
}

fn run_shell(
    input: &mut impl BufRead,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        write!(output, "{PROMPT}")?;
        output.flush()?;
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let text = match std::str::from_utf8(&line) {
            Ok(text) => text,
            Err(_) => {
                writeln!(errors, "Некорректный ввод: ожидается UTF-8")?;
                continue;
            }
        };
        match parse_line(text) {
            Ok(Input::Empty) => {}
            Ok(Input::Exit) => break,
            Ok(Input::Command(spec)) => run_command(spec, errors)?,
            Ok(Input::Pipe(left, right)) => run_pipe(left, right, errors)?,
            Err(error) => writeln!(errors, "Ошибка команды: {error}")?,
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    run_shell(
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeChild {
        kill_error: Option<io::ErrorKind>,
        wait_error: Option<io::ErrorKind>,
        calls: Vec<&'static str>,
    }

    impl ChildCleanup for FakeChild {
        fn kill(&mut self) -> io::Result<()> {
            self.calls.push("kill");
            self.kill_error
                .map_or(Ok(()), |kind| Err(io::Error::new(kind, "kill failed")))
        }

        fn wait(&mut self) -> io::Result<()> {
            self.calls.push("wait");
            self.wait_error
                .map_or(Ok(()), |kind| Err(io::Error::new(kind, "wait failed")))
        }
    }

    #[test]
    fn cleanup_returns_errors_and_always_waits() {
        use io::ErrorKind::{Other, PermissionDenied};

        for (kill_error, wait_error, expected_error) in [
            (None, None, None),
            (Some(PermissionDenied), None, Some(PermissionDenied)),
            (None, Some(Other), Some(Other)),
            (Some(PermissionDenied), Some(Other), Some(PermissionDenied)),
        ] {
            let mut child = FakeChild {
                kill_error,
                wait_error,
                ..FakeChild::default()
            };
            let result = stop_and_wait(&mut child);
            assert_eq!(child.calls, ["kill", "wait"]);
            assert_eq!(result.err().map(|error| error.kind()), expected_error);
        }
    }

    #[test]
    fn cleanup_preserves_both_failures() {
        let mut child = FakeChild {
            kill_error: Some(io::ErrorKind::PermissionDenied),
            wait_error: Some(io::ErrorKind::Other),
            ..FakeChild::default()
        };
        let error = stop_and_wait(&mut child).expect_err("both cleanup operations fail");
        assert_eq!(child.calls, ["kill", "wait"]);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("kill failed"), "{error}");
        assert!(error.to_string().contains("wait failed"), "{error}");
    }

    #[test]
    fn combined_cleanup_keeps_errors_from_both_children() {
        let mut consumer = FakeChild {
            kill_error: Some(io::ErrorKind::PermissionDenied),
            ..FakeChild::default()
        };
        let mut producer = FakeChild {
            wait_error: Some(io::ErrorKind::Other),
            ..FakeChild::default()
        };
        let consumer_cleanup = stop_and_wait(&mut consumer);
        let producer_cleanup = stop_and_wait(&mut producer);
        let error = combine_results(consumer_cleanup, producer_cleanup)
            .expect_err("both children have cleanup failures");
        assert_eq!(consumer.calls, ["kill", "wait"]);
        assert_eq!(producer.calls, ["kill", "wait"]);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("kill failed"), "{error}");
        assert!(error.to_string().contains("wait failed"), "{error}");
    }

    #[test]
    fn failed_diagnostics_keep_initiating_and_cleanup_errors() {
        struct BrokenDiagnostics;

        impl Write for BrokenDiagnostics {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "stderr failed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let error = report_failure(
            &mut BrokenDiagnostics,
            "spawn failed",
            Err(io::Error::other("cleanup failed")),
        )
        .expect_err("diagnostics and cleanup both fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        for message in ["spawn failed", "stderr failed", "cleanup failed"] {
            assert!(error.to_string().contains(message), "{error}");
        }
    }

    #[test]
    fn parses_single_command_and_arguments() {
        assert_eq!(
            parse_line("  echo  hello  world "),
            Ok(Input::Command(CommandSpec {
                program: "echo",
                args: vec!["hello", "world"],
            }))
        );
    }

    #[test]
    fn parses_a_pipe_with_or_without_spaces() {
        assert_eq!(
            parse_line("cat file.txt|grep rust"),
            Ok(Input::Pipe(
                CommandSpec {
                    program: "cat",
                    args: vec!["file.txt"]
                },
                CommandSpec {
                    program: "grep",
                    args: vec!["rust"]
                },
            ))
        );
    }

    #[test]
    fn rejects_malformed_pipelines_and_accepts_empty_input() {
        assert_eq!(parse_line(" \t "), Ok(Input::Empty));
        assert_eq!(parse_line("exit\r\n"), Ok(Input::Exit));
        assert!(parse_line("| cat").is_err());
        assert!(parse_line("cat |").is_err());
        assert!(parse_line("cat | grep | sort").is_err());
    }
}
