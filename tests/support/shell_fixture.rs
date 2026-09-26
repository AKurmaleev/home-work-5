use std::{
    env,
    io::{self, Read, Write},
    process,
};

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("emit") => writeln!(
            io::stdout().lock(),
            "{}",
            args.collect::<Vec<_>>().join(" ")
        )?,
        Some("upper") => {
            let mut input = io::stdin().lock();
            let mut output = io::stdout().lock();
            let mut buffer = [0; 8192];
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                buffer[..count].make_ascii_uppercase();
                output.write_all(&buffer[..count])?;
            }
        }
        Some("count") => {
            let count = io::copy(&mut io::stdin().lock(), &mut io::sink())?;
            writeln!(io::stdout().lock(), "{count}")?;
        }
        Some("large") => io::stdout().lock().write_all(&vec![b'x'; 262_144])?,
        Some("fail") => process::exit(7),
        _ => process::exit(2),
    }
    Ok(())
}
