//! Embeds the Windows application icon and version metadata into `capin.exe`,
//! and rejects locale resources whose keys have drifted.
//!
//! The locale check parses `locales/*.conf` with the same line format as
//! `src/i18n.rs`, compares every language against `en.conf`, and scans
//! `src/**/*.rs` for the literal keys passed to `t()`. Unreferenced keys are
//! reported as warnings instead of build failures so a translation can land
//! before the UI that uses it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/capin_icon.ico");
    println!("cargo:rerun-if-changed=../../locales");
    println!("cargo:rerun-if-changed=src");

    check_locales();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        embed();
    }
}

struct Locale {
    name: String,
    keys: BTreeMap<String, usize>,
}

struct Analysis {
    masked: String,
    literals: Vec<String>,
}

fn check_locales() {
    let crate_root = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let locale_dir = crate_root.join("../../locales");
    let mut errors = Vec::new();

    let mut paths = match fs::read_dir(&locale_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("conf"))
            .collect::<Vec<_>>(),
        Err(error) => {
            errors.push(format!("cannot read {}: {error}", locale_dir.display()));
            Vec::new()
        }
    };
    paths.sort();

    let mut locales = Vec::new();
    for path in &paths {
        if let Some(locale) = read_locale(path, &mut errors) {
            locales.push(locale);
        }
    }

    let Some(reference) = locales.iter().find(|locale| locale.name == "en.conf") else {
        errors.push("locales/en.conf is required as the reference locale".to_owned());
        fail(errors);
        return;
    };

    for locale in &locales {
        if locale.name == reference.name {
            continue;
        }
        for key in reference.keys.keys() {
            if !locale.keys.contains_key(key) {
                errors.push(format!("locales/{}: missing key '{key}'", locale.name));
            }
        }
        for key in locale.keys.keys() {
            if !reference.keys.contains_key(key) {
                errors.push(format!(
                    "locales/{}: key '{key}' does not exist in en.conf",
                    locale.name
                ));
            }
        }
    }

    let source_dir = crate_root.join("src");
    let mut sources = Vec::new();
    collect_sources(&source_dir, &mut sources, &mut errors);
    sources.sort();

    let mut referenced = BTreeSet::new();
    for path in &sources {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                errors.push(format!("cannot read {}: {error}", path.display()));
                continue;
            }
        };
        let analysis = analyze(&source);
        for literal in &analysis.literals {
            if reference.keys.contains_key(literal.as_str()) {
                referenced.insert(literal.clone());
            }
        }
        let relative = path.strip_prefix(&crate_root).unwrap_or(path);
        for (offset, key) in call_keys(&analysis.masked, &source) {
            if !reference.keys.contains_key(key.as_str()) {
                errors.push(format!(
                    "{}:{}: key '{key}' is not defined",
                    relative.display(),
                    line_at(&source, offset)
                ));
            }
        }
    }

    for key in reference.keys.keys() {
        if !referenced.contains(key) {
            println!("cargo:warning=unused locale key '{key}'");
        }
    }

    fail(errors);
}

fn read_locale(path: &Path, errors: &mut Vec<String>) -> Option<Locale> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            errors.push(format!("cannot read {}: {error}", path.display()));
            return None;
        }
    };
    let mut keys = BTreeMap::new();
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            errors.push(format!("locales/{name}:{}: expected key=value", index + 1));
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            errors.push(format!("locales/{name}:{}: empty key", index + 1));
            continue;
        }
        if let Some(first) = keys.insert(key.to_owned(), index + 1) {
            errors.push(format!(
                "locales/{name}:{}: duplicate key '{key}' (first defined at line {first})",
                index + 1
            ));
        }
    }
    Some(Locale { name, keys })
}

fn collect_sources(dir: &Path, sources: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("cannot read {}: {error}", dir.display()));
            return;
        }
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, sources, errors);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
}

fn fail(errors: Vec<String>) {
    if errors.is_empty() {
        return;
    }
    for error in &errors {
        println!("cargo::error={error}");
    }
    std::process::exit(1);
}

fn line_at(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset.min(source.len())]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1
}

/// Masks comments and string contents (keeping quote and newline bytes) so
/// call sites can be located without matching text inside strings or comments.
fn analyze(source: &str) -> Analysis {
    let bytes = source.as_bytes();
    let mut masked = Vec::with_capacity(bytes.len());
    let mut literals = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    masked.push(b' ');
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let mut depth = 0;
                while index < bytes.len() {
                    if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                        depth += 1;
                        masked.extend_from_slice(b"  ");
                        index += 2;
                    } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                        depth -= 1;
                        masked.extend_from_slice(b"  ");
                        index += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        masked.push(if bytes[index] == b'\n' { b'\n' } else { b' ' });
                        index += 1;
                    }
                }
            }
            b'"' => {
                masked.push(b'"');
                index += 1;
                let mut content = Vec::new();
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => {
                            let Some(&next) = bytes.get(index + 1) else {
                                index += 1;
                                break;
                            };
                            content.push(next);
                            masked.push(b' ');
                            masked.push(if next == b'\n' { b'\n' } else { b' ' });
                            index += 2;
                        }
                        b'"' => {
                            masked.push(b'"');
                            index += 1;
                            break;
                        }
                        byte => {
                            content.push(byte);
                            masked.push(if byte == b'\n' { b'\n' } else { b' ' });
                            index += 1;
                        }
                    }
                }
                literals.push(String::from_utf8_lossy(&content).into_owned());
            }
            b'r' if raw_string_start(bytes, index).is_some() => {
                let Some((hashes, quote_offset)) = raw_string_start(bytes, index) else {
                    unreachable!();
                };
                let mut end = bytes.len();
                let mut cursor = index + quote_offset + 1;
                while cursor < bytes.len() {
                    if raw_string_end(bytes, cursor, hashes) {
                        end = cursor + 1 + hashes;
                        break;
                    }
                    cursor += 1;
                }
                for &byte in &bytes[index..end] {
                    masked.push(if byte == b'\n' { b'\n' } else { b' ' });
                }
                index = end;
            }
            b'\'' => {
                if let Some(length) = char_literal_length(bytes, index) {
                    masked.resize(masked.len() + length, b' ');
                    index += length;
                } else {
                    masked.push(b'\'');
                    index += 1;
                }
            }
            byte => {
                masked.push(byte);
                index += 1;
            }
        }
    }

    Analysis {
        masked: String::from_utf8_lossy(&masked).into_owned(),
        literals,
    }
}

fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    let mut cursor = index + 1;
    let mut hashes = 0;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some((hashes, cursor - index))
}

fn raw_string_end(bytes: &[u8], quote: usize, hashes: usize) -> bool {
    bytes.get(quote) == Some(&b'"')
        && (0..hashes).all(|offset| bytes.get(quote + 1 + offset) == Some(&b'#'))
}

fn char_literal_length(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start + 1) == Some(&b'\\') {
        let mut index = start + 3;
        while index < bytes.len() && index < start + 12 {
            match bytes[index] {
                b'\'' => return Some(index + 1 - start),
                b'\n' => return None,
                _ => index += 1,
            }
        }
        None
    } else if bytes.get(start + 2) == Some(&b'\'') {
        Some(3)
    } else {
        None
    }
}

/// Finds string-literal keys passed as the second argument of `t(language, key)`.
fn call_keys(masked: &str, source: &str) -> Vec<(usize, String)> {
    let bytes = masked.as_bytes();
    let source_bytes = source.as_bytes();
    let mut keys = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b't'
            || (index > 0 && is_identifier_byte(bytes[index - 1]))
            || bytes.get(index + 1) != Some(&b'(')
        {
            index += 1;
            continue;
        }
        let mut depth = 1usize;
        let mut argument = 0usize;
        let mut cursor = index + 2;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                b',' if depth == 1 => argument += 1,
                b'"' if depth == 1 && argument == 1 => {
                    if let Some(key) = literal_at(source_bytes, cursor) {
                        keys.push((cursor, key));
                    }
                    break;
                }
                _ => {}
            }
            cursor += 1;
        }
        index = cursor.max(index + 1);
    }
    keys
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
}

fn literal_at(source: &[u8], quote: usize) -> Option<String> {
    let mut content = Vec::new();
    let mut index = quote + 1;
    while index < source.len() {
        match source[index] {
            b'\\' => {
                content.push(*source.get(index + 1)?);
                index += 2;
            }
            b'"' => return Some(String::from_utf8_lossy(&content).into_owned()),
            b'\n' => return None,
            byte => {
                content.push(byte);
                index += 1;
            }
        }
    }
    None
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
