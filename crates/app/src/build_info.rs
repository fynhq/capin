const BUILD_ID: Option<&str> = option_env!("CAPIN_BUILD_ID");

pub(crate) fn version_label() -> String {
    match BUILD_ID {
        Some(id) => format!("Capin v{}+{id}", env!("CARGO_PKG_VERSION")),
        None => format!("Capin v{}", env!("CARGO_PKG_VERSION")),
    }
}
