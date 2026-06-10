use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use std::thread;

/// Run a build function in a loop, re-triggering it whenever `watch_files` change.
///
/// `build_fn` is called immediately, then again on every detected file change.
/// It returns `true` if the build succeeded, `false` on failure.
///
/// Polls every `POLL_MS` milliseconds. Prints a compact status line on each rebuild.
pub fn watch_loop(watch_files: Vec<PathBuf>, mut build_fn: impl FnMut() -> bool) {
    const POLL_MS: u64 = 300;

    println!("Watching {} file(s) for changes — press Ctrl-C to stop.", watch_files.len());

    // Initial build
    let ok = build_fn();
    print_status(ok, None);

    let mut mtimes = snapshot_mtimes(&watch_files);

    loop {
        thread::sleep(Duration::from_millis(POLL_MS));
        let current = snapshot_mtimes(&watch_files);

        if current != mtimes {
            // Find which file changed for the status line
            let changed = watch_files.iter().find(|f| {
                current.get(*f) != mtimes.get(*f)
            }).map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string());

            let t0 = Instant::now();
            let ok = build_fn();
            let elapsed = t0.elapsed();
            print_status(ok, Some((changed.as_deref().unwrap_or("?"), elapsed)));
            mtimes = current;
        }
    }
}

fn snapshot_mtimes(files: &[PathBuf]) -> std::collections::HashMap<PathBuf, Option<SystemTime>> {
    files.iter().map(|f| {
        let mtime = std::fs::metadata(f).and_then(|m| m.modified()).ok();
        (f.clone(), mtime)
    }).collect()
}

fn print_status(ok: bool, rebuild: Option<(&str, Duration)>) {
    match rebuild {
        None => {
            if ok {
                eprintln!("[watch] initial build succeeded — waiting for changes");
            } else {
                eprintln!("[watch] initial build FAILED — waiting for changes");
            }
        }
        Some((file, elapsed)) => {
            let ms = elapsed.as_millis();
            if ok {
                eprintln!("[watch] {} changed → rebuilt in {}ms", file, ms);
            } else {
                eprintln!("[watch] {} changed → build FAILED ({}ms)", file, ms);
            }
        }
    }
}
