//! Embeds the Windows application icon and version metadata into `capin.exe`.
//!
//! `winresource` runs the Windows resource compiler, so this only runs when
//! the build host is Windows. Cross-checks from other hosts compile the same
//! Rust code without producing a Windows resource.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/capin_icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        embed();
    }
}

#[cfg(windows)]
fn embed() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/icon/capin_icon.ico");
    resource.set("ProductName", "Capin");
    resource.set("FileDescription", "Capin");
    resource.set("CompanyName", "Capin");
    resource.set("LegalCopyright", "Copyright (c) 2026 Capin");
    resource.set("OriginalFilename", "capin.exe");
    resource.set("FileVersion", &version);
    resource.set("ProductVersion", &version);
    let words = version_words(&version);
    resource.set_version_info(winresource::VersionInfo::FILEVERSION, words);
    resource.set_version_info(winresource::VersionInfo::PRODUCTVERSION, words);
    resource
        .compile()
        .expect("failed to embed the Windows icon and version resources");
}

#[cfg(windows)]
fn version_words(version: &str) -> u64 {
    let numeric = version
        .split(['-', '+'])
        .next()
        .unwrap_or(version)
        .trim_start_matches('v');
    let mut parts = numeric
        .split('.')
        .map(|part| part.parse::<u16>().unwrap_or(0));
    let major = u64::from(parts.next().unwrap_or(0));
    let minor = u64::from(parts.next().unwrap_or(0));
    let patch = u64::from(parts.next().unwrap_or(0));
    let build = u64::from(parts.next().unwrap_or(0));
    (major << 48) | (minor << 32) | (patch << 16) | build
}
