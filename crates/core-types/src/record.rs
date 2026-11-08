/// Container format for screen recordings. Audio is intentionally unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordFormat {
    Mp4,
    WebP,
}

impl RecordFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::WebP => "webp",
        }
    }

    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Mp4 => "video/mp4",
            Self::WebP => "image/webp",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::WebP => "WebP",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_expose_container_metadata() {
        assert_eq!(RecordFormat::Mp4.extension(), "mp4");
        assert_eq!(RecordFormat::Mp4.mime_type(), "video/mp4");
        assert_eq!(RecordFormat::Mp4.display_name(), "MP4");
        assert_eq!(RecordFormat::WebP.extension(), "webp");
        assert_eq!(RecordFormat::WebP.mime_type(), "image/webp");
        assert_eq!(RecordFormat::WebP.display_name(), "WebP");
    }
}
