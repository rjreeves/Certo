use std::path::Path;
use std::process::{self, Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// CREATE_NO_WINDOW — process gets no console, so no taskbar entry.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Config {
    script:   String,
    minimize: bool,
    hide:     bool,
}

const BANNER: &str = concat!(
    "XEQ v1.0.0.", env!("XEQ_BUILD_DATE"), ".", env!("XEQ_BUILD_NUMBER"), "  (c) SyntrA 2026"
);

fn main() {
    println!("{}", BANNER);

    let args: Vec<String> = std::env::args().collect();

    // Config file defaults to "xeq.cfg" in the current directory.
    let cfg_path = args.get(1).map(String::as_str).unwrap_or("xeq.cfg");

    let raw = std::fs::read_to_string(cfg_path).unwrap_or_else(|e| {
        eprintln!("error: cannot read config '{}': {}", cfg_path, e);
        process::exit(1);
    });

    let cfg = parse_config(&raw).unwrap_or_else(|| {
        eprintln!("error: config '{}' must contain a line like:", cfg_path);
        eprintln!("  script   = C:\\path\\to\\script.ps1");
        eprintln!("  minimize = true          # optional — pwsh only");
        eprintln!("  hide     = true          # optional — no taskbar entry");
        process::exit(1);
    });

    let ext = Path::new(&cfg.script)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    println!("Launching: {}", cfg.script);

    let mut cmd = match ext.as_str() {
        "ps1" => {
            let mut c = Command::new("pwsh");
            c.arg("-ExecutionPolicy").arg("Bypass");
            // minimize and hide are mutually exclusive; hide wins.
            if cfg.minimize && !cfg.hide {
                c.arg("-WindowStyle").arg("Minimized");
            }
            c.arg("-File").arg(&cfg.script);
            c
        }
        "zen" => {
            let mut c = Command::new("zen");
            c.args(["run", &cfg.script, "--yes"]);
            c
        }
        other => {
            eprintln!("error: unknown script type '.{}' — expected .ps1 or .zen", other);
            process::exit(1);
        }
    };

    #[cfg(windows)]
    if cfg.hide {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn().unwrap_or_else(|e| {
        eprintln!("error: failed to launch '{}': {}", cfg.script, e);
        process::exit(1);
    });
}

fn parse_config(raw: &str) -> Option<Config> {
    let mut script:   Option<String> = None;
    let mut minimize: bool           = false;
    let mut hide:     bool           = false;

    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let val = val.trim();
            match key.trim() {
                "script"   => script   = Some(val.to_string()),
                "minimize" => minimize = matches!(val, "true" | "1" | "yes"),
                "hide"     => hide     = matches!(val, "true" | "1" | "yes"),
                _          => {}
            }
        }
    }

    Some(Config { script: script?, minimize, hide })
}
