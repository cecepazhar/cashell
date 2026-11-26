//! Build metadata, plus the Windows icon and version resource for the .exe.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");

    // Source tarballs have no .git: fall back rather than fail the build.
    // The build number is the commit count (a shallow clone reports a lower one).
    let build_number = git(&["rev-list", "--count", "HEAD"])
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let mut commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    if git(&["status", "--porcelain", "--untracked-files=no"]).is_some() {
        commit.push_str("-dirty");
    }
    let built_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    println!("cargo:rustc-env=CASHELL_BUILD_NUMBER={build_number}");
    println!("cargo:rustc-env=CASHELL_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=CASHELL_BUILT_AT={built_at}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resource(build_number, &commit);
    }
}

fn embed_windows_resource(build_number: u16, commit: &str) {
    let part = |name: &str| -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let major = part("CARGO_PKG_VERSION_MAJOR");
    let minor = part("CARGO_PKG_VERSION_MINOR");
    let patch = part("CARGO_PKG_VERSION_PATCH");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();

    // Windows version numbers are four u16 fields: major.minor.patch.build.
    let numeric = (major << 48) | (minor << 32) | (patch << 16) | u64::from(build_number);
    let file_version = format!("{major}.{minor}.{patch}.{build_number}");

    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icon.ico")
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
        .set("FileVersion", &file_version)
        .set("ProductVersion", &format!("{version}+{commit}"))
        .set("ProductName", "CAShell")
        .set("FileDescription", "CAShell - CA ecosystem CLI")
        .set("CompanyName", "Cecep Saeful Azhar Hidayat, ST")
        .set(
            "LegalCopyright",
            "Cecep Saeful Azhar Hidayat, ST - MIT License",
        )
        .set("InternalName", "cashell")
        .set("OriginalFilename", "cashell.exe");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Windows icon/version resource not embedded: {e}");
    }
}
