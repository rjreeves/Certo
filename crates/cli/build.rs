fn main() {
    // Build date as yymmdd from git log, fallback to compile-time env.
    let date = run_git(&["log", "-1", "--format=%cd", "--date=format:%y%m%d"])
        .unwrap_or_else(|| {
            // Try SOURCE_DATE_EPOCH (reproducible builds), otherwise "000000".
            std::env::var("CERTO_BUILD_DATE_OVERRIDE").unwrap_or_else(|_| "000000".to_string())
        });

    // Build number = total commit count on HEAD.
    let num = run_git(&["rev-list", "--count", "HEAD"])
        .unwrap_or_else(|| "0".to_string());

    println!("cargo:rustc-env=CERTO_BUILD_DATE={}", date.trim());
    println!("cargo:rustc-env=CERTO_BUILD_NUM={}", num.trim());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}

fn run_git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8(out.stdout).ok()?;
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    } else {
        None
    }
}
