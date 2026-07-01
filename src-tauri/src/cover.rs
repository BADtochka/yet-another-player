use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const ID3_HEADER_SIZE: usize = 10;
const METADATA_CHUNK_SIZE: usize = 1024 * 1024;

fn sync_safe_size(bytes: &[u8], offset: usize) -> usize {
    ((bytes[offset] as usize) << 21)
        | ((bytes[offset + 1] as usize) << 14)
        | ((bytes[offset + 2] as usize) << 7)
        | (bytes[offset + 3] as usize)
}

fn uint32_size(bytes: &[u8], offset: usize) -> usize {
    ((bytes[offset] as usize) << 24)
        | ((bytes[offset + 1] as usize) << 16)
        | ((bytes[offset + 2] as usize) << 8)
        | (bytes[offset + 3] as usize)
}

fn text_terminator_length(encoding: u8) -> usize {
    if encoding == 1 || encoding == 2 {
        2
    } else {
        1
    }
}

fn find_terminator(bytes: &[u8], offset: usize, encoding: u8) -> Option<usize> {
    let terminator_length = text_terminator_length(encoding);
    let mut index = offset;

    while index + terminator_length <= bytes.len() {
        if terminator_length == 2 {
            if bytes[index] == 0 && bytes[index + 1] == 0 {
                return Some(index);
            }
        } else if bytes[index] == 0 {
            return Some(index);
        }

        index += terminator_length;
    }

    None
}

fn extension_for_mime(mime: &str) -> &'static str {
    if mime.contains("png") {
        "png"
    } else if mime.contains("webp") {
        "webp"
    } else {
        "jpg"
    }
}

fn content_type_for_extension(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "image/jpeg",
    }
}

pub fn cache_file_for_path(cache_dir: &Path, track_path: &Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    track_path.to_string_lossy().hash(&mut hasher);
    let hash = hasher.finish();

    cache_dir.join(format!("{hash:x}.cover"))
}

pub fn read_cached_cover(cache_file: &Path) -> Option<(Vec<u8>, &'static str)> {
    let bytes = fs::read(cache_file).ok()?;
    if bytes.len() < 2 {
        return None;
    }

    let extension_len = bytes[0] as usize;
    if bytes.len() < 1 + extension_len {
        return None;
    }

    let extension = std::str::from_utf8(&bytes[1..1 + extension_len]).ok()?;
    let image = bytes[1 + extension_len..].to_vec();

    if image.is_empty() {
        return None;
    }

    Some((image, content_type_for_extension(extension)))
}

pub fn write_cached_cover(cache_file: &Path, mime: &str, image: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = cache_file
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let extension = extension_for_mime(mime);
    let mut payload = Vec::with_capacity(1 + extension.len() + image.len());
    payload.push(extension.len() as u8);
    payload.extend_from_slice(extension.as_bytes());
    payload.extend_from_slice(image);
    fs::write(cache_file, payload)
}

fn parse_apic_frame(frame: &[u8]) -> Option<(Vec<u8>, String)> {
    if frame.is_empty() {
        return None;
    }

    let encoding = frame[0];
    let mut offset = 1;
    let mime_end = find_terminator(frame, offset, 0)?;
    let mime = std::str::from_utf8(&frame[offset..mime_end])
        .unwrap_or("image/jpeg")
        .to_string();
    offset = mime_end + 1;

    if offset >= frame.len() {
        return None;
    }

    offset += 1;
    let description_end = find_terminator(frame, offset, encoding)?;
    offset = description_end + text_terminator_length(encoding);
    let image = frame.get(offset..)?.to_vec();

    if image.is_empty() {
        return None;
    }

    Some((image, mime))
}

fn parse_pic_frame(frame: &[u8]) -> Option<(Vec<u8>, String)> {
    if frame.len() < 5 {
        return None;
    }

    let encoding = frame[0];
    let format = std::str::from_utf8(&frame[1..4]).unwrap_or("JPG");
    let mime = if format.trim().eq_ignore_ascii_case("PNG") {
        "image/png".to_string()
    } else {
        "image/jpeg".to_string()
    };
    let mut offset = 5;
    let description_end = find_terminator(frame, offset, encoding)?;
    offset = description_end + text_terminator_length(encoding);
    let image = frame.get(offset..)?.to_vec();

    if image.is_empty() {
        return None;
    }

    Some((image, mime))
}

fn frame_size_v2_2(bytes: &[u8], offset: usize) -> usize {
    ((bytes[offset] as usize) << 16) | ((bytes[offset + 1] as usize) << 8) | bytes[offset + 2] as usize
}

fn extract_cover_v2_2(bytes: &[u8], tag_end: usize) -> Option<(Vec<u8>, String)> {
    let mut offset = 0;

    while offset + 6 <= bytes.len() && offset < tag_end {
        let frame_id = std::str::from_utf8(&bytes[offset..offset + 3]).ok()?;
        let frame_size = frame_size_v2_2(bytes, offset + 3);

        if frame_size == 0 || !frame_id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            break;
        }

        let frame_start = offset + 6;
        let frame_end = frame_start + frame_size;
        if frame_end > bytes.len() || frame_end > tag_end {
            break;
        }

        if frame_id == "PIC" {
            return parse_pic_frame(&bytes[frame_start..frame_end]);
        }

        offset = frame_end;
    }

    None
}

fn skip_extended_header(bytes: &[u8], version: u8, offset: usize) -> usize {
    if bytes.len() <= 5 || (bytes[5] & 0x40) == 0 || offset + 4 > bytes.len() {
        return offset;
    }

    let ext_size = if version == 4 {
        sync_safe_size(bytes, offset)
    } else {
        uint32_size(bytes, offset)
    };

    if ext_size >= 4 {
        offset + ext_size
    } else {
        offset
    }
}

pub fn extract_cover_from_mp3(path: &Path) -> Option<(Vec<u8>, String)> {
    let mut file = File::open(path).ok()?;
    let mut buffer = vec![0_u8; METADATA_CHUNK_SIZE];
    let bytes_read = file.read(&mut buffer).ok()?;
    let bytes = &buffer[..bytes_read];

    if bytes.len() < ID3_HEADER_SIZE || bytes[0] != b'I' || bytes[1] != b'D' || bytes[2] != b'3' {
        return None;
    }

    let version = bytes[3];
    let tag_end = ID3_HEADER_SIZE + sync_safe_size(bytes, 6);
    let offset = skip_extended_header(bytes, version, ID3_HEADER_SIZE);

    if version == 2 {
        return extract_cover_v2_2(&bytes[offset..], tag_end.saturating_sub(offset));
    }

    let mut offset = offset;

    while offset + 10 <= bytes.len() && offset + 10 <= tag_end {
        let frame_id = std::str::from_utf8(&bytes[offset..offset + 4]).ok()?;
        let frame_size = if version == 4 {
            sync_safe_size(bytes, offset + 4)
        } else {
            uint32_size(bytes, offset + 4)
        };

        if frame_size == 0 || !frame_id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            break;
        }

        let frame_start = offset + 10;
        let frame_end = frame_start + frame_size;
        if frame_end > bytes.len() || frame_end > tag_end {
            break;
        }

        if frame_id == "APIC" {
            return parse_apic_frame(&bytes[frame_start..frame_end]);
        }

        offset = frame_end;
    }

    None
}

pub fn load_cover(path: &Path, cache_dir: &Path) -> Option<(Vec<u8>, &'static str)> {
    let (image, mime) = if cache_dir.as_os_str().is_empty() {
        extract_cover_from_mp3(path)?
    } else {
        let cache_file = cache_file_for_path(cache_dir, path);

        if let Some(cached) = read_cached_cover(&cache_file) {
            return Some(cached);
        }

        let extracted = extract_cover_from_mp3(path)?;
        let _ = write_cached_cover(&cache_file, &extracted.1, &extracted.0);
        extracted
    };

    Some((image, content_type_for_extension(extension_for_mime(&mime))))
}
