//! Adding songs to the library: from an audio file, or from a YouTube link
//! through the bundled yt-dlp and ffmpeg.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::decode::{DecodeError, decode_to_wav, wav_waveform};
use super::library::{AUDIO_FILE, Library, MANIFEST_VERSION, Manifest, SongSource, now_ms};
use super::youtube::{
    DownloadFailure, ProcessControl, Tools, YtDlpLine, classify_failure, ffmpeg_command,
    parse_ffmpeg_progress, parse_youtube_link, parse_yt_dlp_line, run_process, yt_dlp_command,
};

/// File extensions the importer accepts.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "m4a", "aac", "ogg", "oga"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStage {
    Download,
    Extract,
    Decode,
}

/// Sent while an import runs. `progress` is 0.0..=1.0 when known.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub stage: ImportStage,
    pub progress: Option<f32>,
}

/// Why an import failed (`{ code }` in the webview).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum ImportError {
    #[error("not a YouTube video link")]
    InvalidLink,
    #[error("yt-dlp or ffmpeg is not bundled")]
    ToolsMissing,
    #[error("no network")]
    Network,
    #[error("the video cannot be downloaded")]
    Unavailable,
    #[error("not a supported audio file")]
    UnsupportedFile,
    #[error("another import is running")]
    Busy,
    #[error("cancelled")]
    Cancelled,
    #[error("import failed: {message}")]
    Failed { message: String },
}

impl ImportError {
    fn failed(message: impl ToString) -> Self {
        Self::Failed {
            message: message.to_string(),
        }
    }
}

impl From<DecodeError> for ImportError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::Cancelled => Self::Cancelled,
            DecodeError::NoAudio | DecodeError::Unsupported(_) => Self::UnsupportedFile,
            other => Self::failed(other),
        }
    }
}

/// Removes an unfinished import directory unless the import completes.
struct Staging {
    dir: PathBuf,
    done: bool,
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.done {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled".to_owned())
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Copies `source` into the library and decodes its playback copy.
pub fn import_file(
    library: &Library,
    source: &Path,
    control: &ProcessControl,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<Manifest, ImportError> {
    let extension = extension_of(source)
        .filter(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
        .ok_or(ImportError::UnsupportedFile)?;
    if !source.is_file() {
        return Err(ImportError::UnsupportedFile);
    }
    let (id, dir) = library.begin_import().map_err(ImportError::failed)?;
    let mut staging = Staging { dir, done: false };
    let original = format!("original.{extension}");
    fs::copy(source, staging.dir.join(&original)).map_err(ImportError::failed)?;
    progress(ImportProgress {
        stage: ImportStage::Decode,
        progress: Some(0.0),
    });
    let decoded = decode_to_wav(
        &staging.dir.join(&original),
        &staging.dir.join(AUDIO_FILE),
        control.cancel_flag(),
        &mut |done| {
            progress(ImportProgress {
                stage: ImportStage::Decode,
                progress: Some(done),
            })
        },
    )?;
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        id,
        title: title_from_path(source),
        source: SongSource::File {
            file_name: source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        original,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        frames: decoded.frames,
        added_at_ms: now_ms(),
        stems: Vec::new(),
    };
    library
        .finish_import(&staging.dir, &manifest, &decoded)
        .map_err(ImportError::failed)?;
    staging.done = true;
    Ok(manifest)
}

/// Downloads the audio of a YouTube link, extracts it with ffmpeg and adds
/// it to the library.
pub fn import_link(
    library: &Library,
    input: &str,
    tools: Option<Tools>,
    cache: &Path,
    control: &ProcessControl,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<Manifest, ImportError> {
    let link = parse_youtube_link(input).ok_or(ImportError::InvalidLink)?;
    let tools = tools.ok_or(ImportError::ToolsMissing)?;
    let (id, dir) = library.begin_import().map_err(ImportError::failed)?;
    let mut staging = Staging { dir, done: false };

    progress(ImportProgress {
        stage: ImportStage::Download,
        progress: None,
    });
    let mut downloaded: Option<PathBuf> = None;
    let mut title = String::new();
    let mut duration: Option<f64> = None;
    let outcome = run_process(
        yt_dlp_command(&tools, &link, &staging.dir, cache),
        control,
        &mut |line| match parse_yt_dlp_line(line) {
            YtDlpLine::Progress(done) => progress(ImportProgress {
                stage: ImportStage::Download,
                progress: Some(done),
            }),
            YtDlpLine::File(path) => downloaded = Some(path),
            YtDlpLine::Info {
                duration: seconds,
                title: name,
            } => {
                duration = seconds;
                title = name;
            }
            YtDlpLine::Other => {}
        },
    )
    .map_err(ImportError::failed)?;
    if control.cancelled() {
        return Err(ImportError::Cancelled);
    }
    if !outcome.success {
        log::warn!("yt-dlp failed: {}", outcome.stderr.trim());
        return Err(match classify_failure(&outcome.stderr) {
            DownloadFailure::Network => ImportError::Network,
            DownloadFailure::Unavailable => ImportError::Unavailable,
            DownloadFailure::Other => ImportError::failed(outcome.stderr.trim()),
        });
    }
    let downloaded = downloaded
        .filter(|path| path.is_file())
        .ok_or_else(|| ImportError::failed("yt-dlp reported no file"))?;

    progress(ImportProgress {
        stage: ImportStage::Extract,
        progress: Some(0.0),
    });
    let audio = staging.dir.join(AUDIO_FILE);
    let outcome = run_process(
        ffmpeg_command(&tools, &downloaded, &audio),
        control,
        &mut |line| {
            if let (Some(done), Some(total)) = (parse_ffmpeg_progress(line), duration)
                && total > 0.0
            {
                progress(ImportProgress {
                    stage: ImportStage::Extract,
                    progress: Some((done / total).clamp(0.0, 1.0) as f32),
                });
            }
        },
    )
    .map_err(ImportError::failed)?;
    if control.cancelled() {
        return Err(ImportError::Cancelled);
    }
    if !outcome.success {
        log::warn!("ffmpeg failed: {}", outcome.stderr.trim());
        return Err(ImportError::failed(outcome.stderr.trim()));
    }
    let decoded = wav_waveform(&audio)?;
    if decoded.frames == 0 {
        return Err(ImportError::Unavailable);
    }

    let original = format!(
        "original.{}",
        extension_of(&downloaded).unwrap_or_else(|| "bin".into())
    );
    fs::rename(&downloaded, staging.dir.join(&original)).map_err(ImportError::failed)?;
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        id,
        title: if title.is_empty() {
            link.video_id.clone()
        } else {
            title
        },
        source: SongSource::Youtube {
            url: link.url(),
            video_id: link.video_id,
        },
        original,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        frames: decoded.frames,
        added_at_ms: now_ms(),
        stems: Vec::new(),
    };
    library
        .finish_import(&staging.dir, &manifest, &decoded)
        .map_err(ImportError::failed)?;
    staging.done = true;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("woodshed-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..4_800 {
            writer.write_sample(((i % 100) * 100) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn imported_file_lands_in_the_library() {
        let dir = temp_dir("file");
        let source = dir.join("My Song.WAV");
        write_wav(&source);
        let library = Library::new(&dir.join("data"));
        let mut stages = Vec::new();
        let song = import_file(&library, &source, &ProcessControl::default(), &mut |p| {
            stages.push(p.stage)
        })
        .unwrap();
        assert_eq!(song.title, "My Song");
        assert_eq!(song.original, "original.wav");
        assert_eq!(
            (song.sample_rate, song.channels, song.frames),
            (48_000, 1, 4_800)
        );
        assert_eq!(library.list(), vec![song.clone()]);
        let song_dir = library.song_dir(&song.id).unwrap();
        assert!(song_dir.join("original.wav").is_file());
        assert!(song_dir.join(AUDIO_FILE).is_file());
        assert!(stages.contains(&ImportStage::Decode));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_files_leave_nothing_behind() {
        let dir = temp_dir("unsupported");
        let library = Library::new(&dir.join("data"));
        let text = dir.join("notes.txt");
        fs::write(&text, b"hi").unwrap();
        let control = ProcessControl::default();
        assert_eq!(
            import_file(&library, &text, &control, &mut |_| {}),
            Err(ImportError::UnsupportedFile)
        );
        let fake = dir.join("fake.mp3");
        fs::write(&fake, b"not audio").unwrap();
        assert_eq!(
            import_file(&library, &fake, &control, &mut |_| {}),
            Err(ImportError::UnsupportedFile)
        );
        assert!(library.list().is_empty());
        let incoming = library.root().join(super::super::library::INCOMING_DIR);
        assert_eq!(fs::read_dir(incoming).map(|d| d.count()).unwrap_or(0), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Stand-ins for yt-dlp and ffmpeg that speak the same output protocol.
    fn fake_tools(dir: &Path, source: &Path, download_ok: bool) -> Tools {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let download = if download_ok {
            format!(
                r#"file=$(echo "$out" | sed 's/%(ext)s/webm/')
cp "{}" "$file"
echo "WSPROG 50 100 NA"
echo "WSPROG 100 100 NA"
echo "WSFILE $file"
echo "WSINFO 0.1 Fake Title""#,
                source.display()
            )
        } else {
            "echo 'ERROR: [youtube] x: Video unavailable' >&2\nexit 1".to_owned()
        };
        script(
            &bin.join("yt-dlp"),
            &format!(
                r#"out=""
while [ $# -gt 0 ]; do case "$1" in -o) out="$2"; shift;; esac; shift; done
{download}"#
            ),
        );
        script(
            &bin.join("ffmpeg"),
            r#"input=""; for last; do :; done
while [ $# -gt 0 ]; do case "$1" in -i) input="$2"; shift;; esac; shift; done
cp "$input" "$last"
echo "out_time_us=50000"
echo "progress=end""#,
        );
        Tools::in_dir(&bin).unwrap()
    }

    #[test]
    fn link_import_downloads_extracts_and_adds_the_song() {
        let dir = temp_dir("fake-tools");
        let source = dir.join("source.wav");
        write_wav(&source);
        let tools = fake_tools(&dir, &source, true);
        let library = Library::new(&dir.join("data"));
        let mut reports = Vec::new();
        let song = import_link(
            &library,
            "https://youtu.be/jNQXAC9IVRw?si=x",
            Some(tools),
            &dir,
            &ProcessControl::default(),
            &mut |p| reports.push(p),
        )
        .unwrap();
        assert_eq!(song.title, "Fake Title");
        assert_eq!(
            song.source,
            SongSource::Youtube {
                url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".into(),
                video_id: "jNQXAC9IVRw".into()
            }
        );
        assert_eq!(song.original, "original.webm");
        assert_eq!(song.frames, 4_800);
        assert!(reports.contains(&ImportProgress {
            stage: ImportStage::Download,
            progress: Some(0.5)
        }));
        assert!(reports.contains(&ImportProgress {
            stage: ImportStage::Extract,
            progress: Some(0.5)
        }));
        assert_eq!(library.list(), vec![song]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_download_reports_why_and_cleans_up() {
        let dir = temp_dir("fake-fail");
        let source = dir.join("source.wav");
        write_wav(&source);
        let tools = fake_tools(&dir, &source, false);
        let library = Library::new(&dir.join("data"));
        let result = import_link(
            &library,
            "https://youtu.be/jNQXAC9IVRw",
            Some(tools),
            &dir,
            &ProcessControl::default(),
            &mut |_| {},
        );
        assert_eq!(result, Err(ImportError::Unavailable));
        assert!(library.list().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn links_are_checked_before_tools() {
        let dir = temp_dir("link");
        let library = Library::new(&dir.join("data"));
        let control = ProcessControl::default();
        assert_eq!(
            import_link(
                &library,
                "https://example.com/x",
                None,
                &dir,
                &control,
                &mut |_| {}
            ),
            Err(ImportError::InvalidLink)
        );
        assert_eq!(
            import_link(
                &library,
                "https://youtu.be/jNQXAC9IVRw",
                None,
                &dir,
                &control,
                &mut |_| {}
            ),
            Err(ImportError::ToolsMissing)
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
