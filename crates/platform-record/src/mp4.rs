//! Post-processing for MP4 recordings: drops the empty audio track the Windows
//! transcoder leaves behind and writes the encoder and author URL into the
//! iTunes-style `moov.udta.meta.ilst` box that Windows Explorer and most
//! players expose as file properties.
//!
//! Both platform encoders finalize `moov` after `mdat`, so the new metadata is
//! appended inside the last top-level box. A file whose `moov` is not last is
//! rejected instead of rewriting chunk offsets.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const ENCODER: &str = "Capin";
const PROJECT_URL: &str = "https://github.com/fynhq/capin";

const A9: u8 = 0xA9;
const TAG_ENCODER: [u8; 4] = [A9, b'e', b'n', b'c'];
const TAG_URL: [u8; 4] = [A9, b'u', b'r', b'l'];
const TAGS: [([u8; 4], &str); 2] = [(TAG_ENCODER, ENCODER), (TAG_URL, PROJECT_URL)];

#[derive(Clone, Copy)]
struct BoxSpan {
    kind: [u8; 4],
    start: usize,
    header: usize,
    size: usize,
}

impl BoxSpan {
    fn payload_range(self) -> std::ops::Range<usize> {
        self.start + self.header..self.start + self.size
    }
}

#[allow(dead_code)] // only the Windows and macOS recorders call this
pub(crate) fn inject_metadata(path: &Path) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("open recording: {error}"))?;
    let length = file
        .metadata()
        .map_err(|error| format!("read recording size: {error}"))?
        .len() as usize;
    let boxes = scan_boxes(&mut file, length)?;
    let Some(moov) = boxes.iter().find(|entry| &entry.kind == b"moov").copied() else {
        return Err("recording has no moov box".to_owned());
    };
    if moov.start + moov.size != length {
        return Err("moov is not the last box; metadata needs chunk offset fixups".to_owned());
    }

    let mut payload = vec![0u8; moov.size - moov.header];
    file.seek(SeekFrom::Start(moov.payload_range().start as u64))
        .map_err(|error| format!("seek to moov: {error}"))?;
    file.read_exact(&mut payload)
        .map_err(|error| format!("read moov: {error}"))?;
    let payload = update_moov(&payload)?;

    file.seek(SeekFrom::Start(moov.start as u64))
        .map_err(|error| format!("seek to moov start: {error}"))?;
    file.write_all(&make_box(b"moov", &payload))
        .map_err(|error| format!("write moov: {error}"))?;
    file.set_len((moov.start + 8 + payload.len()) as u64)
        .map_err(|error| format!("truncate recording: {error}"))?;
    Ok(())
}

fn scan_boxes<F: Read + Seek>(file: &mut F, length: usize) -> Result<Vec<BoxSpan>, String> {
    let mut boxes = Vec::new();
    let mut offset = 0usize;
    while offset + 8 <= length {
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|error| format!("seek to box: {error}"))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)
            .map_err(|error| format!("read box header: {error}"))?;
        let mut size = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
        let mut header_len = 8usize;
        if size == 1 {
            let mut extended = [0u8; 8];
            file.read_exact(&mut extended)
                .map_err(|error| format!("read box size: {error}"))?;
            size = u64::from_be_bytes(extended) as usize;
            header_len = 16;
        } else if size == 0 {
            size = length - offset;
        }
        if size < header_len || offset.saturating_add(size) > length {
            return Err("recording has an invalid box size".to_owned());
        }
        boxes.push(BoxSpan {
            kind: header[4..8].try_into().unwrap(),
            start: offset,
            header: header_len,
            size,
        });
        offset += size;
    }
    Ok(boxes)
}

fn parse_boxes(bytes: &[u8]) -> Result<Vec<BoxSpan>, String> {
    let mut boxes = Vec::new();
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let header = 8usize;
        if size < header || offset + size > bytes.len() {
            return Err("metadata box has an invalid size".to_owned());
        }
        boxes.push(BoxSpan {
            kind: bytes[offset + 4..offset + 8].try_into().unwrap(),
            start: offset,
            header,
            size,
        });
        offset += size;
    }
    if offset != bytes.len() && bytes[offset..].iter().any(|byte| *byte != 0) {
        return Err("metadata has trailing bytes".to_owned());
    }
    Ok(boxes)
}

fn update_moov(payload: &[u8]) -> Result<Vec<u8>, String> {
    let children = parse_boxes(payload)?;
    // The Windows transcoder always declares an audio stream even when audio
    // is disabled. Its track has zero samples but makes strict players reject
    // the file, so drop empty tracks while keeping at least one real track.
    let empty_traks: Vec<bool> = children
        .iter()
        .map(|child| {
            &child.kind == b"trak" && track_sample_count(&payload[child.payload_range()]) == Some(0)
        })
        .collect();
    let non_empty_traks = children
        .iter()
        .zip(&empty_traks)
        .filter(|(child, empty)| &child.kind == b"trak" && !**empty)
        .count();

    let mut rebuilt = Vec::with_capacity(payload.len() + 160);
    let mut replaced = false;
    for (child, empty) in children.iter().zip(empty_traks) {
        if empty && non_empty_traks > 0 {
            continue;
        }
        if &child.kind == b"udta" {
            let udta = update_udta(&payload[child.payload_range()])?;
            rebuilt.extend_from_slice(&make_box(b"udta", &udta));
            replaced = true;
        } else {
            rebuilt.extend_from_slice(&payload[child.start..child.start + child.size]);
        }
    }
    if !replaced {
        let ilst = update_ilst(&[])?;
        rebuilt.extend_from_slice(&make_box(b"udta", &make_meta(&ilst)));
    }
    Ok(rebuilt)
}

fn track_sample_count(trak: &[u8]) -> Option<u32> {
    let mdia = child_payload(trak, b"mdia")?;
    let minf = child_payload(mdia, b"minf")?;
    let stbl = child_payload(minf, b"stbl")?;
    let stsz = child_payload(stbl, b"stsz")?;
    (stsz.len() >= 12).then(|| u32::from_be_bytes(stsz[8..12].try_into().unwrap()))
}

fn child_payload<'a>(payload: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let span = parse_boxes(payload)
        .ok()?
        .into_iter()
        .find(|entry| &entry.kind == kind)?;
    Some(&payload[span.payload_range()])
}

fn update_udta(payload: &[u8]) -> Result<Vec<u8>, String> {
    let children = parse_boxes(payload)?;
    let mut rebuilt = Vec::with_capacity(payload.len() + 160);
    let mut replaced = false;
    for child in &children {
        if &child.kind == b"meta" {
            let meta = update_meta(&payload[child.payload_range()])?;
            rebuilt.extend_from_slice(&make_box(b"meta", &meta));
            replaced = true;
        } else {
            rebuilt.extend_from_slice(&payload[child.start..child.start + child.size]);
        }
    }
    if !replaced {
        let ilst = update_ilst(&[])?;
        rebuilt.extend_from_slice(&make_meta(&ilst));
    }
    Ok(rebuilt)
}

fn update_meta(payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() < 4 {
        return Err("metadata meta box is truncated".to_owned());
    }
    let children = parse_boxes(&payload[4..])?;
    let mut rebuilt = payload[..4].to_vec();
    let mut replaced = false;
    for child in &children {
        let start = 4 + child.start;
        if &child.kind == b"ilst" {
            let ilst = update_ilst(&payload[start + child.header..start + child.size])?;
            rebuilt.extend_from_slice(&ilst);
            replaced = true;
        } else {
            rebuilt.extend_from_slice(&payload[start..start + child.size]);
        }
    }
    if !replaced {
        rebuilt.extend_from_slice(&update_ilst(&[])?);
    }
    Ok(rebuilt)
}

fn update_ilst(payload: &[u8]) -> Result<Vec<u8>, String> {
    let items = parse_boxes(payload)?;
    let mut rebuilt = Vec::with_capacity(payload.len() + 160);
    let mut seen = Vec::new();
    for item in &items {
        if let Some((_, value)) = TAGS.iter().find(|(kind, _)| *kind == item.kind) {
            rebuilt.extend_from_slice(&make_item(item.kind, value));
            seen.push(item.kind);
        } else {
            rebuilt.extend_from_slice(&payload[item.start..item.start + item.size]);
        }
    }
    for (kind, value) in TAGS {
        if !seen.contains(&kind) {
            rebuilt.extend_from_slice(&make_item(kind, value));
        }
    }
    Ok(make_box(b"ilst", &rebuilt))
}

fn make_meta(ilst: &[u8]) -> Vec<u8> {
    let mut meta = vec![0u8; 4];
    meta.extend_from_slice(&make_hdlr());
    meta.extend_from_slice(ilst);
    make_box(b"meta", &meta)
}

fn make_hdlr() -> Vec<u8> {
    let mut payload = vec![0u8; 4];
    payload.extend_from_slice(&0u32.to_be_bytes());
    payload.extend_from_slice(b"mdir");
    payload.extend_from_slice(&[0u8; 12]);
    payload.push(0);
    make_box(b"hdlr", &payload)
}

fn make_item(kind: [u8; 4], value: &str) -> Vec<u8> {
    // `data` full box: version/flags 1 (UTF-8), locale 0, then the string.
    let mut data = vec![0u8; 8];
    data[3] = 1;
    data.extend_from_slice(value.as_bytes());
    make_box(&kind, &make_box(b"data", &data))
}

fn make_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = 8 + payload.len();
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&(size as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "capin-mp4-{name}-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn child(bytes: &[u8], kind: &[u8; 4]) -> Option<BoxSpan> {
        parse_boxes(bytes)
            .ok()?
            .into_iter()
            .find(|entry| &entry.kind == kind)
    }

    fn tag_value(moov: &[u8], kind: [u8; 4]) -> Option<String> {
        let udta = child(moov, b"udta")?;
        let udta = &moov[udta.payload_range()];
        let meta = child(udta, b"meta")?;
        let meta = &udta[meta.payload_range()];
        let ilst_span = child(meta.get(4..)?, b"ilst")?;
        let ilst = &meta[4 + ilst_span.start..4 + ilst_span.start + ilst_span.size];
        let items = &ilst[ilst_span.header..];
        let item = child(items, &kind)?;
        let data = child(&items[item.payload_range()], b"data")?;
        let bytes = &items[item.payload_range()][data.payload_range()];
        Some(String::from_utf8_lossy(&bytes[8..]).into_owned())
    }

    #[test]
    fn injects_tags_into_existing_meta() {
        let path = temp_path("existing");
        let ftyp = make_box(b"ftyp", b"isom");
        let mdat = make_box(b"mdat", &[7u8; 32]);
        let moov = make_box(
            b"moov",
            &make_box(b"udta", &make_meta(&update_ilst(&[]).unwrap())),
        );
        let mut file = ftyp;
        file.extend_from_slice(&mdat);
        file.extend_from_slice(&moov);
        fs::write(&path, &file).unwrap();

        inject_metadata(&path).unwrap();
        let patched = fs::read(&path).unwrap();
        let boxes = parse_boxes(&patched).unwrap();
        let moov_box = boxes.last().unwrap();
        assert_eq!(moov_box.kind, *b"moov");
        let moov = &patched[moov_box.payload_range()];
        assert_eq!(tag_value(moov, TAG_ENCODER).as_deref(), Some(ENCODER));
        assert_eq!(tag_value(moov, TAG_URL).as_deref(), Some(PROJECT_URL));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn injecting_twice_does_not_duplicate_items() {
        let path = temp_path("twice");
        let moov = make_box(b"moov", &make_box(b"udta", &make_meta(&[])));
        let mut file = make_box(b"mdat", &[1u8; 8]);
        file.extend_from_slice(&moov);
        fs::write(&path, &file).unwrap();
        inject_metadata(&path).unwrap();
        inject_metadata(&path).unwrap();
        let patched = fs::read(&path).unwrap();
        let boxes = parse_boxes(&patched).unwrap();
        let moov = &patched[boxes.last().unwrap().payload_range()];
        let udta = child(moov, b"udta").unwrap();
        let udta = &moov[udta.payload_range()];
        let meta = child(udta, b"meta").unwrap();
        let meta = &udta[meta.payload_range()];
        let ilst_span = child(&meta[4..], b"ilst").unwrap();
        let ilst = &meta[4 + ilst_span.start..4 + ilst_span.start + ilst_span.size];
        let count = parse_boxes(&ilst[ilst_span.header..])
            .unwrap()
            .iter()
            .filter(|entry| entry.kind == TAG_ENCODER)
            .count();
        assert_eq!(count, 1);
        let _ = fs::remove_file(&path);
    }

    fn track(sample_count: u32) -> Vec<u8> {
        let mut stsz = vec![0u8; 8];
        stsz.extend_from_slice(&sample_count.to_be_bytes());
        let stbl = make_box(b"stbl", &make_box(b"stsz", &stsz));
        let minf = make_box(b"minf", &stbl);
        let mdia = make_box(b"mdia", &minf);
        make_box(b"trak", &mdia)
    }

    #[test]
    fn drops_empty_audio_track_and_keeps_video() {
        let path = temp_path("audio");
        let mut moov_payload = track(3);
        moov_payload.extend_from_slice(&track(0));
        moov_payload.extend_from_slice(&make_box(b"udta", &make_meta(&update_ilst(&[]).unwrap())));
        let mut file = make_box(b"mdat", &[3u8; 8]);
        file.extend_from_slice(&make_box(b"moov", &moov_payload));
        fs::write(&path, &file).unwrap();

        inject_metadata(&path).unwrap();
        let patched = fs::read(&path).unwrap();
        let boxes = parse_boxes(&patched).unwrap();
        let moov = &patched[boxes.last().unwrap().payload_range()];
        let traks: Vec<BoxSpan> = parse_boxes(moov)
            .unwrap()
            .into_iter()
            .filter(|entry| &entry.kind == b"trak")
            .collect();
        assert_eq!(traks.len(), 1);
        assert_eq!(track_sample_count(&moov[traks[0].payload_range()]), Some(3));
        assert_eq!(tag_value(moov, TAG_URL).as_deref(), Some(PROJECT_URL));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_moov_before_mdat_without_touching_the_file() {
        let path = temp_path("faststart");
        let moov = make_box(b"moov", &[]);
        let mdat = make_box(b"mdat", &[2u8; 8]);
        let mut file = moov;
        file.extend_from_slice(&mdat);
        fs::write(&path, &file).unwrap();
        assert!(inject_metadata(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), file);
        let _ = fs::remove_file(&path);
    }
}
