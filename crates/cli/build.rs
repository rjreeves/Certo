fn main() {
    // Build date as yymmdd.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (yy, mm, dd) = unix_to_ymd(now);
    let date = format!("{:02}{:02}{:02}", yy % 100, mm, dd);

    // Auto-incrementing build counter — increments on every build.
    println!("cargo:rerun-if-changed=build_number.txt");
    let counter_path = "build_number.txt";
    let build_num: u32 = std::fs::read_to_string(counter_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
        + 1;
    std::fs::write(counter_path, build_num.to_string())
        .expect("failed to write build_number.txt");

    // Embed FileVersion/ProductVersion/ProductName into the .exe's Windows
    // Properties dialog. Must be gated to Windows only: `WindowsResource::
    // compile()` shells out to `windres`/`rc.exe`, which don't exist on
    // Linux/macOS — calling it unconditionally would break the build there.
    // (`winres` itself, as a build-dependency, compiles fine cross-platform;
    // it's only *invoking* it that's Windows-only.)
    if cfg!(windows) {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let parts: Vec<u64> = version
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        let major = *parts.first().unwrap_or(&0);
        let minor = *parts.get(1).unwrap_or(&0);
        let patch = *parts.get(2).unwrap_or(&0);
        let numeric_version = (major << 48) | (minor << 32) | (patch << 16);

        let mut resource = winres::WindowsResource::new();
        // `WindowsResource::new()` already defaults FileVersion/
        // ProductVersion to CARGO_PKG_VERSION and ProductName/
        // FileDescription to CARGO_PKG_NAME ("certo") — override the two
        // that should read as human-facing product text, not the crate's
        // lowercase package name.
        resource.set("FileVersion", &version);
        resource.set("ProductVersion", &version);
        resource.set("ProductName", "Certo Compiler");
        resource.set("FileDescription", "Certo Compiler");
        resource.set_version_info(winres::VersionInfo::FILEVERSION, numeric_version);
        resource.set_version_info(winres::VersionInfo::PRODUCTVERSION, numeric_version);

        // Actually embed the resource — building it above with no `compile()`
        // call is a no-op; every field set is silently discarded otherwise.
        // Not `.unwrap()`: a missing `rc.exe`/Windows SDK on some contributor's
        // machine shouldn't hard-fail the whole build over cosmetic .exe
        // Properties metadata — warn and continue instead.
        if let Err(e) = resource.compile() {
            println!("cargo:warning=failed to embed Windows version info: {}", e);
        }
    }

    println!("cargo:rustc-env=CERTO_BUILD_DATE={}", date);
    println!("cargo:rustc-env=CERTO_BUILD_NUM={}", build_num);
    println!("cargo:rerun-if-changed=build.rs");
}

fn unix_to_ymd(secs: u64) -> (u32, u32, u32) {
    let days = (secs / 86400) as u32;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y   = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp  = (5 * doy + 2) / 153;
    let d   = doy - (153 * mp + 2) / 5 + 1;
    let m   = if mp < 10 { mp + 3 } else { mp - 9 };
    let y   = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
