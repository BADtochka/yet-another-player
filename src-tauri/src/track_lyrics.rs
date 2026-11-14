use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    panic::catch_unwind,
    path::Path,
};

use lofty::{
    config::{ParseOptions, ParsingMode, WriteOptions},
    file::TaggedFileExt,
    id3::v2::Id3v2Tag,
    mp4::{AtomIdent, Ilst},
    ogg::tag::VorbisComments,
    probe::Probe,
    tag::{ItemKey, Tag, TagExt, TagItem, TagType},
};
use tauri::{AppHandle, Emitter, State};

use crate::{
    audio::library_track_path,
    library::{Library, LibraryResult},
    MusicFolders,
};

const LYRICS_KEYS: [ItemKey; 2] = [ItemKey::Lyrics, ItemKey::UnsyncLyrics];

#[tauri::command]
pub(crate) async fn save_track_lyrics(
    app: AppHandle,
    library: State<'_, Library>,
    folders: State<'_, MusicFolders>,
    path: String,
    text: String,
    background: Option<bool>,
    expected_fetched: Option<String>,
) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > 256 * 1024 || text.contains('\0') {
        return Err("Недопустимый текст песни".into());
    }
    let path = library_track_path(&path, &folders.canonical)?;
    let database = library.database_path().to_owned();
    let writer = library.lock_catalog_writer().await;
    tauri::async_runtime::spawn_blocking(move || -> LibraryResult<()> {
        let _writer = writer;
        if background == Some(true) && crate::library::read_fetched_lyrics(&database, &path)? != expected_fetched {
            return Ok(());
        }
        crate::library::save_fetched_lyrics(&database, &path, &text)?;
        // Unsupported formats and files with existing lyrics keep the fetched version in the catalog.
        match catch_unwind(|| embed(&path, &text)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("Lyrics tags: {error}"),
            Err(panic) => eprintln!("Lyrics tags: {}", crate::panic_message(&*panic)),
        }
        if background == Some(true) {
            let _ = app.emit("library:lyrics", path);
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| {
        eprintln!("Lyrics catalog: {error}");
        "Не удалось сохранить текст песни".into()
    })
}

fn embed(path: &Path, text: &str) -> LibraryResult<()> {
    let mut source = File::open(path)?;
    let metadata = source.metadata()?;
    let modified = metadata.modified()?;
    let parse = ParseOptions::new()
        .read_properties(false)
        .parsing_mode(ParsingMode::Strict);
    let original = Probe::new(&mut source)
        .guess_file_type()?
        .options(parse)
        .read()?;
    if original.tags().iter().any(has_lyrics) {
        return Ok(());
    }
    let kind = original.primary_tag_type();
    let key = match kind {
        TagType::Id3v2 => ItemKey::UnsyncLyrics,
        TagType::Mp4Ilst | TagType::VorbisComments => ItemKey::Lyrics,
        _ => return Ok(()),
    };
    // A new primary tag would hide metadata held by a secondary tag in the library.
    if original.primary_tag().is_none() && !original.tags().is_empty() {
        return Ok(());
    }
    let mut tag = original
        .primary_tag()
        .cloned()
        .unwrap_or_else(|| Tag::new(kind));
    if !tag.insert_text(key, text.to_owned()) {
        return Err("Format cannot store lyrics".into());
    }

    let parent = path.parent().ok_or("Missing track directory")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    source.seek(SeekFrom::Start(0))?;
    std::io::copy(&mut source, staged.as_file_mut())?;
    staged.as_file_mut().seek(SeekFrom::Start(0))?;
    let mut header = [0; 4];
    let v23 = staged.as_file_mut().read_exact(&mut header).is_ok() && header == *b"ID3\x03";
    staged.as_file_mut().seek(SeekFrom::Start(0))?;
    let options = WriteOptions::new().use_id3v23(v23).parse_options(parse);
    tag.save_to(staged.as_file_mut(), options)?;
    staged.as_file_mut().seek(SeekFrom::Start(0))?;
    let written = Probe::new(staged.as_file_mut())
        .guess_file_type()?
        .options(parse)
        .read()?;
    let same_metadata = original.tags().iter().all(|before| {
        written.tag(before.tag_type()).is_some_and(|after| {
            before.pictures() == after.pictures()
                && other_items(before) == other_items(after)
                && native_tags_unchanged(before, after)
        })
    });
    if !same_metadata || written.primary_tag().and_then(|tag| tag.get_string(key)) != Some(text) {
        return Err("Lyrics write failed metadata verification".into());
    }
    let latest = path.metadata()?;
    if latest.modified()? != modified || latest.len() != metadata.len() {
        return Err("Track changed while saving lyrics".into());
    }
    staged.as_file().set_permissions(metadata.permissions())?;
    staged.as_file().sync_all()?;
    drop(source);
    staged.persist(path)?;
    Ok(())
}

fn has_lyrics(tag: &Tag) -> bool {
    LYRICS_KEYS.iter().any(|key| tag.get(*key).is_some())
        || (tag.tag_type() == TagType::Id3v2
            && (&Id3v2Tag::from(tag.clone()))
                .into_iter()
                .any(|frame| frame.id_str() == "SYLT"))
}

fn other_items(tag: &Tag) -> HashMap<ItemKey, Vec<&TagItem>> {
    let mut fields = HashMap::<_, Vec<_>>::new();
    // Field order can change during serialization. Keep repeated values in their original order.
    for item in tag
        .items()
        .filter(|item| !LYRICS_KEYS.contains(&item.key()))
    {
        fields.entry(item.key()).or_default().push(item);
    }
    fields
}

fn native_tags_unchanged(before: &Tag, after: &Tag) -> bool {
    match before.tag_type() {
        TagType::Id3v2 => {
            let before = Id3v2Tag::from(before.clone());
            let mut after = Id3v2Tag::from(after.clone());
            after
                .remove(&lofty::id3::v2::FrameId::Valid("USLT".into()))
                .for_each(drop);
            let mut previous = (&before).into_iter().collect::<Vec<_>>();
            let mut current = (&after).into_iter().collect::<Vec<_>>();
            previous.sort_by_key(|frame| frame.id_str());
            current.sort_by_key(|frame| frame.id_str());
            before.flags() == after.flags() && previous == current
        }
        TagType::VorbisComments => {
            let before = VorbisComments::from(before.clone());
            let mut after = VorbisComments::from(after.clone());
            after.remove("LYRICS").for_each(drop);
            let mut previous = before.items().collect::<Vec<_>>();
            let mut current = after.items().collect::<Vec<_>>();
            // Vorbis field order can change during serialization. Stable sort keeps duplicate values ordered.
            previous.sort_by_key(|(key, _)| *key);
            current.sort_by_key(|(key, _)| *key);
            before.vendor() == after.vendor() && previous == current
        }
        TagType::Mp4Ilst => {
            let before = Ilst::from(before.clone());
            let mut after = Ilst::from(after.clone());
            after.remove(&AtomIdent::Fourcc(*b"\xa9lyr")).for_each(drop);
            before == after
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use lofty::tag::Accessor;

    use super::*;

    #[test]
    fn embeds_unicode_id3_lyrics_without_changing_other_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.wav");
        std::fs::write(
            &path,
            b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x02\0\0\0\0\0",
        ).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Название 日本語".into());
        tag.set_artist("Исполнитель".into());
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        embed(&path, "Текст 🐈").unwrap();
        let file = Probe::open(&path).unwrap().read().unwrap();
        let tag = file.tag(TagType::Id3v2).unwrap();
        assert_eq!(tag.get_string(ItemKey::UnsyncLyrics), Some("Текст 🐈"));
        assert_eq!(tag.artist().as_deref(), Some("Исполнитель"));
    }

    #[test]
    fn preserves_unicode_tags_and_the_first_embedded_lyrics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.opus");
        std::fs::write(&path, include_bytes!("library/fixtures/one-sample.opus")).unwrap();

        let parse = ParseOptions::new().read_properties(false);
        let mut tag = Probe::open(&path)
            .unwrap()
            .options(parse)
            .read()
            .unwrap()
            .primary_tag()
            .unwrap()
            .clone();
        tag.set_title("Заголовок 日本語".into());
        tag.insert_text(ItemKey::Comment, "Примечание ñ".into());
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        let lyrics = "[0:01.027]Музыка 🎵 日本語";
        embed(&path, lyrics).unwrap();
        let after = std::fs::read(&path).unwrap();
        embed(&path, "Другая версия").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), after);

        let written = Probe::open(&path).unwrap().options(parse).read().unwrap();
        let tag = written.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("Заголовок 日本語"));
        assert_eq!(tag.get_string(ItemKey::Comment), Some("Примечание ñ"));
        assert_eq!(tag.get_string(ItemKey::Lyrics), Some(lyrics));
    }
}
