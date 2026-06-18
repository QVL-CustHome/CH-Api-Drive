use chrono::{DateTime, NaiveDateTime, Utc};
use image::DynamicImage;
use std::io::Cursor;

const THUMB_MAX: u32 = 320;

pub struct ImageMeta {
    pub width: i32,
    pub height: i32,
    pub taken_at: Option<DateTime<Utc>>,
    pub thumbnail: Vec<u8>,
}

pub fn process_image(bytes: &[u8]) -> Option<ImageMeta> {
    let exif = read_exif(bytes);
    let taken_at = exif.as_ref().and_then(read_taken_at);
    let orientation = exif.as_ref().and_then(read_orientation).unwrap_or(1);

    let img = image::load_from_memory(bytes).ok()?;
    let (raw_w, raw_h) = (img.width(), img.height());

    let oriented = apply_orientation(img, orientation);
    let thumb = oriented.thumbnail(THUMB_MAX, THUMB_MAX);

    let mut out = Vec::new();
    thumb
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .ok()?;

    let (width, height) = match orientation {
        5 | 6 | 7 | 8 => (raw_h, raw_w),
        _ => (raw_w, raw_h),
    };

    Some(ImageMeta {
        width: width as i32,
        height: height as i32,
        taken_at,
        thumbnail: out,
    })
}

fn read_exif(bytes: &[u8]) -> Option<exif::Exif> {
    let mut cursor = Cursor::new(bytes);
    exif::Reader::new().read_from_container(&mut cursor).ok()
}

fn read_taken_at(exif: &exif::Exif) -> Option<DateTime<Utc>> {
    let field = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
    let raw = field.display_value().to_string();
    let trimmed = raw.trim().trim_matches('"');
    NaiveDateTime::parse_from_str(trimmed, "%Y:%m:%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc())
}

fn read_orientation(exif: &exif::Exif) -> Option<u32> {
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)
}

fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}
