pub(super) fn qr_url(code: &str) -> Option<String> {
    let code = code.trim();
    if code.is_empty() || code.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = code.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(code.to_owned());
    }
    if lower.starts_with("www.") {
        return Some(format!("https://{code}"));
    }
    if bare_domain(code) {
        return Some(format!("https://{code}"));
    }
    None
}

fn bare_domain(code: &str) -> bool {
    let authority = code.split(['/', '?', '#']).next().unwrap_or(code);
    let host = authority.split(':').next().unwrap_or(authority);
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    labels.next().is_some() && tld.len() >= 2 && tld.chars().all(|ch| ch.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::qr_url;

    #[test]
    fn absolute_links_pass_through() {
        assert_eq!(
            qr_url("https://example.com/a?b=1"),
            Some("https://example.com/a?b=1".to_owned())
        );
        assert_eq!(
            qr_url("HTTP://example.com"),
            Some("HTTP://example.com".to_owned())
        );
    }

    #[test]
    fn bare_domains_get_https() {
        assert_eq!(
            qr_url("www.example.com"),
            Some("https://www.example.com".to_owned())
        );
        assert_eq!(
            qr_url("example.com/path"),
            Some("https://example.com/path".to_owned())
        );
        assert_eq!(
            qr_url("192.168.1.10:8080/ui"),
            Some("https://192.168.1.10:8080/ui".to_owned())
        );
    }

    #[test]
    fn plain_text_is_not_a_link() {
        assert_eq!(qr_url("hello world"), None);
        assert_eq!(qr_url("2024-01-01"), None);
        assert_eq!(qr_url("v1.2.3"), None);
    }
}
