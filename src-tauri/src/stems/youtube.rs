//! YouTube links: parsing, the bundled sidecars (yt-dlp, ffmpeg) and running
//! them with progress and cancellation.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// A YouTube video link reduced to its video id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YoutubeLink {
    pub video_id: String,
}

impl YoutubeLink {
    /// The canonical watch URL (drops playlist and tracking parameters).
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.video_id)
    }
}

fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Accepts `youtube.com/watch?v=`, `youtu.be/`, `/shorts/`, `/live/` and
/// `/embed/` links, with or without `https://`, `www.`, `m.` or `music.`.
pub fn parse_youtube_link(input: &str) -> Option<YoutubeLink> {
    let input = input.trim();
    let lower = input.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &input[8..]
    } else if lower.starts_with("http://") {
        &input[7..]
    } else if lower.contains("://") {
        return None;
    } else {
        input
    };
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..split].to_ascii_lowercase();
    let host = ["www.", "m.", "music."]
        .iter()
        .find_map(|prefix| host.strip_prefix(prefix))
        .unwrap_or(&host);
    let tail = &rest[split..];
    let (path, query) = match tail.split_once('?') {
        Some((path, query)) => (path, query.split('#').next().unwrap_or("")),
        None => (tail.split('#').next().unwrap_or(""), ""),
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let id = match host {
        "youtu.be" => segments.first().copied(),
        "youtube.com" | "youtube-nocookie.com" => match segments.as_slice() {
            ["watch"] => query.split('&').find_map(|pair| pair.strip_prefix("v=")),
            ["shorts" | "live" | "embed" | "v", id, ..] => Some(*id),
            _ => None,
        },
        _ => None,
    }?;
    valid_video_id(id).then(|| YoutubeLink {
        video_id: id.to_owned(),
    })
}

/// The bundled sidecars, next to the app executable.
#[derive(Clone, Debug)]
pub struct Tools {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
}

impl Tools {
    /// Finds the sidecars in `dir`. Empty placeholders (see `build.rs`)
    /// count as missing.
    pub fn in_dir(dir: &Path) -> Option<Self> {
        let usable = |name: &str| {
            let path = dir.join(name);
            std::fs::metadata(&path)
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
                .then_some(path)
        };
        Some(Self {
            yt_dlp: usable("yt-dlp")?,
            ffmpeg: usable("ffmpeg")?,
        })
    }

    pub fn locate() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        Self::in_dir(exe.parent()?)
    }
}

/// A cancellable external process run.
#[derive(Default)]
pub struct ProcessControl {
    cancelled: AtomicBool,
    child: Mutex<Option<u32>>,
}

impl ProcessControl {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancelled
    }

    /// Stops the running process (and everything it started).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let pid = *self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pid) = pid {
            terminate_group(pid);
        }
    }
}

/// Sends SIGTERM to the process group `pid` leads. yt-dlp's single-file build
/// runs a child interpreter, which a plain kill of the launcher would orphan.
fn terminate_group(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[derive(Debug)]
pub struct ProcessOutcome {
    pub success: bool,
    pub stderr: String,
}

/// Runs `command`, calling `on_line` for each stdout line, until it exits
/// or `control` cancels it.
pub fn run_process(
    mut command: Command,
    control: &ProcessControl,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<ProcessOutcome> {
    use std::os::unix::process::CommandExt;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child: Child = command.spawn()?;
    *control.child.lock().unwrap_or_else(PoisonError::into_inner) = Some(child.id());
    if control.cancelled() {
        terminate_group(child.id());
    }
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    });
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            on_line(&line);
        }
    }
    let status = child.wait()?;
    *control.child.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let stderr = stderr
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    Ok(ProcessOutcome {
        success: status.success(),
        stderr,
    })
}

/// Why yt-dlp failed, read from its error output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadFailure {
    Network,
    Unavailable,
    Other,
}

pub fn classify_failure(stderr: &str) -> DownloadFailure {
    const NETWORK: &[&str] = &[
        "Unable to download webpage",
        "Unable to download API page",
        "urlopen error",
        "nodename nor servname",
        "Name or service not known",
        "Network is unreachable",
        "No route to host",
        "timed out",
        "Connection reset",
        "Connection refused",
        "Failed to resolve",
        "getaddrinfo failed",
        "Temporary failure in name resolution",
        "The Internet connection appears to be offline",
    ];
    const UNAVAILABLE: &[&str] = &[
        "Video unavailable",
        "Private video",
        "This video is not available",
        "Incomplete YouTube ID",
        "Unsupported URL",
        "is not a valid URL",
        "This video has been removed",
        "Sign in to confirm your age",
    ];
    if NETWORK.iter().any(|needle| stderr.contains(needle)) {
        DownloadFailure::Network
    } else if UNAVAILABLE.iter().any(|needle| stderr.contains(needle)) {
        DownloadFailure::Unavailable
    } else {
        DownloadFailure::Other
    }
}

/// What a yt-dlp progress or result line says.
#[derive(Clone, Debug, PartialEq)]
pub enum YtDlpLine {
    Progress(f32),
    File(PathBuf),
    Info {
        duration: Option<f64>,
        title: String,
    },
    Other,
}

pub const PROGRESS_TEMPLATE: &str = "download:WSPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s";
pub const FILE_TEMPLATE: &str = "after_move:WSFILE %(filepath)s";
pub const INFO_TEMPLATE: &str = "after_move:WSINFO %(duration)s %(title)s";

pub fn parse_yt_dlp_line(line: &str) -> YtDlpLine {
    if let Some(rest) = line.strip_prefix("WSPROG ") {
        let mut parts = rest.split_whitespace().map(|v| v.parse::<f64>().ok());
        let done = parts.next().flatten();
        let total = parts.next().flatten();
        let estimate = parts.next().flatten();
        return match (done, total.or(estimate)) {
            (Some(done), Some(total)) if total > 0.0 => {
                YtDlpLine::Progress((done / total).clamp(0.0, 1.0) as f32)
            }
            _ => YtDlpLine::Other,
        };
    }
    if let Some(path) = line.strip_prefix("WSFILE ") {
        return YtDlpLine::File(PathBuf::from(path));
    }
    if let Some(rest) = line.strip_prefix("WSINFO ") {
        let (duration, title) = rest.split_once(' ').unwrap_or((rest, ""));
        return YtDlpLine::Info {
            duration: duration.parse().ok(),
            title: title.trim().to_owned(),
        };
    }
    YtDlpLine::Other
}

/// The yt-dlp invocation that downloads the best audio-only stream of
/// `link` into `dir/download.<ext>` without post-processing.
pub fn yt_dlp_command(tools: &Tools, link: &YoutubeLink, dir: &Path, cache: &Path) -> Command {
    let mut command = Command::new(&tools.yt_dlp);
    command
        .arg("--no-playlist")
        .arg("--newline")
        .arg("--no-colors")
        .arg("--no-mtime")
        .arg("--no-part")
        .args(["-f", "bestaudio/best"])
        .arg("-o")
        .arg(dir.join("download.%(ext)s"))
        .arg("--progress")
        .args(["--progress-template", PROGRESS_TEMPLATE])
        .args(["--print", FILE_TEMPLATE])
        .args(["--print", INFO_TEMPLATE])
        .arg("--cache-dir")
        .arg(cache)
        .arg("--ffmpeg-location")
        .arg(&tools.ffmpeg)
        .arg("--")
        .arg(link.url());
    command
}

/// The ffmpeg invocation that extracts the first audio stream of `input` as
/// 16-bit stereo PCM WAV at its own rate, reporting progress on stdout.
pub fn ffmpeg_command(tools: &Tools, input: &Path, output: &Path) -> Command {
    let mut command = Command::new(&tools.ffmpeg);
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-y",
        ])
        .arg("-i")
        .arg(input)
        .args(["-vn", "-map", "0:a:0", "-ac", "2", "-c:a", "pcm_s16le"])
        .args(["-progress", "pipe:1"])
        .arg(output);
    command
}

/// Seconds of output ffmpeg reports in a `-progress` line.
pub fn parse_ffmpeg_progress(line: &str) -> Option<f64> {
    let micros = line
        .strip_prefix("out_time_us=")?
        .trim()
        .parse::<f64>()
        .ok()?;
    (micros >= 0.0).then_some(micros / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links_parse_to_their_video_id() {
        for input in [
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
            "https://youtube.com/watch?feature=share&v=jNQXAC9IVRw&t=10",
            "http://m.youtube.com/watch?v=jNQXAC9IVRw",
            "https://music.youtube.com/watch?v=jNQXAC9IVRw&list=RDAMVM",
            "youtube.com/watch?v=jNQXAC9IVRw",
            "https://youtu.be/jNQXAC9IVRw?si=abc",
            "  https://www.youtube.com/shorts/jNQXAC9IVRw  ",
            "https://www.youtube.com/live/jNQXAC9IVRw?feature=share",
            "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=jNQXAC9IVRw#t=5",
        ] {
            assert_eq!(
                parse_youtube_link(input).map(|link| link.video_id),
                Some("jNQXAC9IVRw".to_owned()),
                "{input}"
            );
        }
    }

    #[test]
    fn other_links_are_rejected() {
        for input in [
            "",
            "hello",
            "https://vimeo.com/123456",
            "https://www.youtube.com/",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/playlist?list=PL123",
            "https://notyoutube.com/watch?v=jNQXAC9IVRw",
            "ftp://youtube.com/watch?v=jNQXAC9IVRw",
            "https://www.youtube.com/watch?v=jNQXAC9IVR!",
        ] {
            assert_eq!(parse_youtube_link(input), None, "{input}");
        }
        assert_eq!(
            parse_youtube_link("youtu.be/jNQXAC9IVRw").unwrap().url(),
            "https://www.youtube.com/watch?v=jNQXAC9IVRw"
        );
    }

    #[test]
    fn yt_dlp_lines_are_understood() {
        assert_eq!(
            parse_yt_dlp_line("WSPROG 50 200 NA"),
            YtDlpLine::Progress(0.25)
        );
        assert_eq!(
            parse_yt_dlp_line("WSPROG 50 NA 100.0"),
            YtDlpLine::Progress(0.5)
        );
        assert_eq!(parse_yt_dlp_line("WSPROG 50 NA NA"), YtDlpLine::Other);
        assert_eq!(
            parse_yt_dlp_line("WSFILE /tmp/a b/download.webm"),
            YtDlpLine::File(PathBuf::from("/tmp/a b/download.webm"))
        );
        assert_eq!(
            parse_yt_dlp_line("WSINFO 19 Me at the zoo"),
            YtDlpLine::Info {
                duration: Some(19.0),
                title: "Me at the zoo".into()
            }
        );
        assert_eq!(
            parse_yt_dlp_line("WSINFO NA Live"),
            YtDlpLine::Info {
                duration: None,
                title: "Live".into()
            }
        );
        assert_eq!(
            parse_ffmpeg_progress("out_time_us=19005542"),
            Some(19.005542)
        );
        assert_eq!(parse_ffmpeg_progress("out_time_us=N/A"), None);
    }

    #[test]
    fn failures_are_classified() {
        assert_eq!(
            classify_failure(
                "ERROR: [youtube] x: Unable to download webpage: <urlopen error [Errno 8] nodename nor servname provided>"
            ),
            DownloadFailure::Network
        );
        assert_eq!(
            classify_failure("ERROR: [youtube] x: Video unavailable"),
            DownloadFailure::Unavailable
        );
        assert_eq!(
            classify_failure("ERROR: something else"),
            DownloadFailure::Other
        );
    }

    #[test]
    fn empty_placeholders_count_as_missing_tools() {
        let dir = std::env::temp_dir().join(format!("woodshed-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("yt-dlp"), b"").unwrap();
        std::fs::write(dir.join("ffmpeg"), b"bin").unwrap();
        assert!(Tools::in_dir(&dir).is_none());
        std::fs::write(dir.join("yt-dlp"), b"bin").unwrap();
        assert!(Tools::in_dir(&dir).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_a_running_process_group() {
        let control = ProcessControl::default();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "echo started; sleep 30 & wait"]);
        let started = std::time::Instant::now();
        let outcome = std::thread::scope(|scope| {
            let runner = scope
                .spawn(|| run_process(command, &control, &mut |line| assert_eq!(line, "started")));
            std::thread::sleep(std::time::Duration::from_millis(300));
            control.cancel();
            runner.join().unwrap().unwrap()
        });
        assert!(!outcome.success);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }
}
