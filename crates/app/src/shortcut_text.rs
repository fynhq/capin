#[cfg(target_os = "macos")]
const PRIMARY: &str = "⌘";
#[cfg(target_os = "macos")]
const SHIFT: &str = "⇧";
#[cfg(target_os = "macos")]
const ALT: &str = "⌥";

pub(crate) fn primary(key: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        format!("{PRIMARY}{key}")
    }
    #[cfg(not(target_os = "macos"))]
    {
        format!("Ctrl+{key}")
    }
}

pub(crate) fn primary_shift(key: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        format!("{SHIFT}{PRIMARY}{key}")
    }
    #[cfg(not(target_os = "macos"))]
    {
        format!("Ctrl+Shift+{key}")
    }
}

pub(crate) fn shift(key: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        format!("{SHIFT}{key}")
    }
    #[cfg(not(target_os = "macos"))]
    {
        format!("Shift+{key}")
    }
}

pub(crate) fn alt(key: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        format!("{ALT}{key}")
    }
    #[cfg(not(target_os = "macos"))]
    {
        format!("Alt+{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_uses_platform_convention() {
        #[cfg(target_os = "macos")]
        assert_eq!(primary("S"), "⌘S");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(primary("S"), "Ctrl+S");
    }

    #[test]
    fn shift_and_alt_keep_the_key_verbatim() {
        let wheel = shift("Wheel");
        let drag = alt("Drag");
        assert!(wheel.ends_with("Wheel"));
        assert!(drag.ends_with("Drag"));
    }
}
