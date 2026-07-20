use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let (paths, check_mode) = parse_args(&args[1..]);

    if paths.is_empty() {
        // Read from stdin, write to stdout
        let mut src = String::new();
        use std::io::Read;
        std::io::stdin().read_to_string(&mut src).unwrap_or_else(|e| {
            eprintln!("error reading stdin: {}", e); process::exit(1);
        });
        match certo_fmt::format_source(&src) {
            Ok(out) => print!("{}", out),
            Err(errs) => {
                for e in &errs { eprintln!("parse error: {}", e); }
                process::exit(1);
            }
        }
        return;
    }

    let mut any_changed = false;
    let mut any_error   = false;

    for path in &paths {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("error reading {}: {}", path.display(), e); process::exit(1);
        });

        match certo_fmt::format_source(&src) {
            Ok(formatted) => {
                if formatted == src {
                    continue;
                }
                any_changed = true;
                if check_mode {
                    eprintln!("{}: would reformat", path.display());
                } else {
                    std::fs::write(path, &formatted).unwrap_or_else(|e| {
                        eprintln!("error writing {}: {}", path.display(), e); process::exit(1);
                    });
                    eprintln!("{}: reformatted", path.display());
                }
            }
            Err(errs) => {
                for e in &errs { eprintln!("{}: parse error: {}", path.display(), e); }
                any_error = true;
            }
        }
    }

    if any_error   { process::exit(1); }
    if check_mode && any_changed { process::exit(2); } // convention: 2 = "needs formatting"
}

fn parse_args(args: &[String]) -> (Vec<PathBuf>, bool) {
    let mut paths    = Vec::new();
    let mut check    = false;
    for arg in args {
        match arg.as_str() {
            "--check" | "-c" => check = true,
            other => paths.push(PathBuf::from(other)),
        }
    }
    (paths, check)
}
