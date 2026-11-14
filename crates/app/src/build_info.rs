//! Build identity shared by the settings footer and diagnostics.

/// Short commit hash injected by dev packaging builds; formal releases leave
/// it unset and show the bare semver version.
const BUILD_ID: Option<&str> = option_env!("CAPIN_BUILD_ID");

pub(crate) fn version_label() -> String {
    match BUILD_ID {
        Some(id) => format!("Capin v{}+{id}", env!("CARGO_PKG_VERSION")),
        None => format!("Capin v{}", env!("CARGO_PKG_VERSION")),
    }
}
