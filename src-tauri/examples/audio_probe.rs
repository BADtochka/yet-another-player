use std::{env, error::Error, io, path::PathBuf, time::Duration};

use rodio::{cpal::BufferSize, DeviceSinkBuilder, Player, Source};

#[path = "../src/audio/source.rs"]
mod source;

const USAGE: &str = "Usage: audio_probe FILE [--decode-only] [--seek SECONDS]";

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic")
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut args = env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or_else(|| invalid_input(USAGE))?);
    let mut seek_to = None;
    let mut decode_only = false;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--decode-only") => decode_only = true,
            Some("--seek") => {
                let seconds: f64 = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or_else(|| invalid_input(USAGE))?
                    .parse()?;
                seek_to = Some(Duration::try_from_secs_f64(seconds)?);
            }
            _ => return Err(invalid_input(USAGE).into()),
        }
    }

    let mut source = source::open(&path)?;
    let duration = source.total_duration();
    if let (Some(seek), Some(total)) = (seek_to, duration) {
        if seek >= total {
            return Err(invalid_input("Seek must be before the end of the file").into());
        }
    }

    println!(
        "{}: {} Hz, {} channels, duration {:?}",
        path.display(),
        source.sample_rate(),
        source.channels(),
        duration
    );

    if decode_only {
        if let Some(position) = seek_to {
            source.try_seek(position)?;
        }
        let sample_count = source.count();
        if sample_count == 0 {
            return Err(invalid_input("Decoder produced no audio samples").into());
        }
        println!("Decoded {sample_count} samples");
        return Ok(());
    }

    play(source, seek_to)
}

fn play(
    source: Box<dyn Source + Send>,
    seek_to: Option<Duration>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Rodio's fixed buffer can exceed the range of some CoreAudio devices.
    let output = DeviceSinkBuilder::from_default_device()?
        .with_buffer_size(BufferSize::Default)
        .open_stream()?;
    let player = Player::connect_new(output.mixer());
    player.set_volume(0.1);
    if seek_to.is_some() {
        player.pause();
    }
    player.append(source);

    if let Some(position) = seek_to {
        player.try_seek(position)?;
        println!("Paused and sought to {:?}", player.get_pos());
        player.play();
    }

    player.sleep_until_end();
    println!("Finished playback");
    Ok(())
}
