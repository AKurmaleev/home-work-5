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

fn stop_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
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
        stop_and_wait(&mut producer);
        return Err(io::Error::other(
            "не удалось получить stdout первого процесса",
        ));
    };
    let mut consumer = match Command::new(right.program)
        .args(right.args)
        .stdin(Stdio::from(reader))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            stop_and_wait(&mut producer);
            return writeln!(errors, "Ошибка запуска '{}': {error}", right.program);
        }
    };

    // The parent owns no read end: EOF and broken-pipe reach the right processes.
    let right_status = match consumer.wait() {
        Ok(status) => status,
        Err(error) => {
            stop_and_wait(&mut consumer);
            stop_and_wait(&mut producer);
            return writeln!(errors, "Ошибка ожидания '{}': {error}", right.program);
        }
    };
    let left_status = match producer.wait() {
        Ok(status) => Some(status),
        Err(error) => {
            stop_and_wait(&mut producer);
            writeln!(errors, "Ошибка ожидания '{}': {error}", left.program)?;
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
