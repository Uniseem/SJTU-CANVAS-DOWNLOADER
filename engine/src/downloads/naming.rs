//! Where a download is saved: a course folder below the chosen folder, one
//! folder per lesson, and names that are valid on both Windows and macOS.
//!
//! ```text
//! <chosen folder>/
//!   常微分方程 [87084]/
//!     课堂录像/
//!       2026-06-18_18-55 第74讲 [1a2b3c4d]/
//!         电脑屏幕.mp4
//!         教室摄像头.mp4
//!     课程文件/
//!       讲义.pdf
//!       讲义 (2).pdf
//! ```
//!
//! Existing files are never overwritten: a taken name gets " (2)", " (3)"….

use std::{
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use unicode_normalization::UnicodeNormalization;

pub const VIDEO_FOLDER: &str = "课堂录像";
pub const FILE_FOLDER: &str = "课程文件";
const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "webm", "m4v", "flv", "mov", "mkv"];

pub fn track_label(track: &str) -> &'static str {
    match track {
        "slides" => "电脑屏幕",
        "teacher" => "教室摄像头",
        "composite" => "合成画面",
        _ => "视频",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub directories: Vec<String>,
    pub filename: String,
}

pub fn video_layout(
    course_name: &str,
    course_id: &str,
    title: &str,
    begin_time: Option<&str>,
    lesson_id: &str,
    track: &str,
    server_filename: &str,
) -> Layout {
    let extension = extension_of(server_filename)
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| VIDEO_EXTENSIONS.contains(&value.as_str()))
        .unwrap_or_else(|| "mp4".into());
    let title = safe_segment(or(title, "课堂录像"), 48);
    let lecture = [
        begin_time.map(lesson_date).unwrap_or_default(),
        format!("{title} [{}]", short_id(lesson_id)),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    Layout {
        directories: vec![
            course_folder(course_name, course_id),
            VIDEO_FOLDER.into(),
            lecture,
        ],
        filename: format!("{}.{extension}", track_label(track)),
    }
}

pub fn file_layout(course_name: &str, course_id: &str, server_filename: &str) -> Layout {
    let extension = extension_of(server_filename).filter(|value| {
        (1..=10).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    let stem = match &extension {
        Some(extension) => &server_filename[..server_filename.len() - extension.len() - 1],
        None => server_filename,
    };
    Layout {
        directories: vec![course_folder(course_name, course_id), FILE_FOLDER.into()],
        filename: match extension {
            Some(extension) => format!("{}.{extension}", safe_segment(stem, 110)),
            None => safe_segment(stem, 110),
        },
    }
}

fn course_folder(course_name: &str, course_id: &str) -> String {
    format!(
        "{} [{}]",
        safe_segment(or(course_name, "课程"), 48),
        safe_segment(or(course_id, "未知"), 20)
    )
}

fn or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn extension_of(filename: &str) -> Option<String> {
    let (stem, extension) = filename.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_string())
}

/// A single path segment: never a path, never a reserved Windows name.
pub fn safe_segment(value: &str, limit: usize) -> String {
    let cleaned = value
        .nfc()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let limited = cleaned.trim().chars().take(limit).collect::<String>();
    let trimmed = limited.trim_end_matches(['.', ' ']);
    let result = if trimmed.is_empty() {
        "未命名"
    } else {
        trimmed
    };
    if is_reserved_name(result) {
        format!("_{result}")
    } else {
        result.to_string()
    }
}

fn is_reserved_name(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or_default().to_lowercase();
    if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") {
        return true;
    }
    let mut characters = stem.chars();
    let prefix = characters.by_ref().take(3).collect::<String>();
    let rest = characters.collect::<Vec<_>>();
    (prefix == "com" || prefix == "lpt")
        && rest.len() == 1
        && matches!(rest[0], '1'..='9' | '¹' | '²' | '³')
}

/// 8 hex digits of FNV-1a over the id, as the web version named lessons.
pub fn short_id(value: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for character in value.chars() {
        let mut units = [0u16; 2];
        let unit = character.encode_utf16(&mut units)[0];
        hash = (hash ^ u32::from(unit)).wrapping_mul(16_777_619);
    }
    format!("{hash:08x}")
}

/// `2026-06-18_18-55` in China Standard Time; empty when unparseable.
pub fn lesson_date(value: &str) -> String {
    let china = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let value = value.trim();
    let parsed = DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&china))
        .ok()
        .or_else(|| {
            [
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%d %H:%M",
                "%Y-%m-%dT%H:%M:%S",
                "%Y-%m-%dT%H:%M",
            ]
            .into_iter()
            .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
            .and_then(|naive| china.from_local_datetime(&naive).single())
        });
    parsed
        .map(|date| date.format("%Y-%m-%d_%H-%M").to_string())
        .unwrap_or_default()
}

/// The partial file of `path` while it downloads: `电脑屏幕.mp4.part`.
pub fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

fn numbered(filename: &str, index: usize) -> String {
    if index == 1 {
        return filename.to_string();
    }
    match filename.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({index}).{extension}"),
        _ => format!("{filename} ({index})"),
    }
}

/// Creates the folders and reserves a free name by creating its partial file.
pub fn allocate(destination: &Path, layout: &Layout) -> io::Result<PathBuf> {
    let mut directory = destination.to_path_buf();
    for segment in &layout.directories {
        directory.push(segment);
    }
    std::fs::create_dir_all(&directory)?;
    for index in 1..=10_000 {
        let path = directory.join(numbered(&layout.filename, index));
        if path.exists() {
            continue;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(part_path(&path))
        {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("同名文件过多，请选择另一个保存文件夹"))
}

/// `讲义 (3).pdf` → (`讲义.pdf`, 3); other names are their own base.
fn base_name(filename: &str) -> (String, usize) {
    let (stem, extension) = match filename.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (filename, None),
    };
    let numbered = stem
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .and_then(|(base, number)| Some((base, number.parse::<usize>().ok()?)))
        .filter(|(base, number)| !base.is_empty() && *number >= 2);
    match (numbered, extension) {
        (Some((base, number)), Some(extension)) => (format!("{base}.{extension}"), number),
        (Some((base, number)), None) => (base.to_string(), number),
        (None, _) => (filename.to_string(), 1),
    }
}

/// Moves the finished partial file of `path` into place. If something else
/// took the name meanwhile, the next free number is used; nothing is replaced.
pub fn finish(path: &Path) -> io::Result<PathBuf> {
    let part = part_path(path);
    if !path.exists() {
        std::fs::rename(&part, path)?;
        return Ok(path.to_path_buf());
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let (base, taken) = base_name(&filename);
    for index in (taken + 1).max(2)..=10_000 {
        let candidate = directory.join(numbered(&base, index));
        if candidate.exists() || part_path(&candidate).exists() {
            continue;
        }
        std::fs::rename(&part, &candidate)?;
        return Ok(candidate);
    }
    Err(io::Error::other("同名文件过多，无法保存下载的文件"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_layout_groups_by_course_and_lesson() {
        let layout = video_layout(
            "常微分方程",
            "87084",
            "第74讲",
            Some("2026-06-18 18:55:00"),
            "lesson-1",
            "slides",
            "第74讲_2026-06-18_PPT_lesson-1.MP4",
        );
        assert_eq!(
            layout.directories,
            vec![
                "常微分方程 [87084]".to_string(),
                "课堂录像".into(),
                format!("2026-06-18_18-55 第74讲 [{}]", short_id("lesson-1")),
            ]
        );
        assert_eq!(layout.filename, "电脑屏幕.mp4");
        let hls = video_layout("课", "1", "讲", None, "x", "teacher", "playlist.m3u8");
        assert_eq!(hls.filename, "教室摄像头.mp4");
        assert_eq!(hls.directories[2], format!("讲 [{}]", short_id("x")));
    }

    #[test]
    fn file_layout_keeps_the_extension_and_cleans_the_stem() {
        let layout = file_layout("机器学习", "42", "第 08 讲: CNN?.pdf");
        assert_eq!(
            layout.directories,
            vec!["机器学习 [42]".to_string(), "课程文件".into()]
        );
        assert_eq!(layout.filename, "第 08 讲_ CNN_.pdf");
        assert_eq!(file_layout("c", "1", "README").filename, "README");
        assert_eq!(
            file_layout("c", "1", "archive.tar.gz").filename,
            "archive.tar.gz"
        );
    }

    #[test]
    fn short_ids_match_the_web_version() {
        // FNV-1a 32-bit, as computed by the former TypeScript implementation.
        assert_eq!(short_id(""), "811c9dc5");
        assert_eq!(short_id("a"), "e40c292c");
        assert_eq!(short_id("foobar"), "bf9cf968");
    }

    #[test]
    fn segments_are_safe_on_windows_and_macos() {
        assert_eq!(safe_segment("../../CON?.mp4", 70), ".._.._CON_.mp4");
        assert_eq!(safe_segment("CON", 70), "_CON");
        assert_eq!(safe_segment("com1.txt", 70), "_com1.txt");
        assert_eq!(safe_segment("com10", 70), "com10");
        assert_eq!(safe_segment("  ", 70), "未命名");
        assert_eq!(safe_segment("结尾. ", 70), "结尾");
        assert_eq!(safe_segment("a\u{0}b\nc", 70), "a_b_c");
        assert_eq!(
            safe_segment("长标题".repeat(40).as_str(), 5)
                .chars()
                .count(),
            5
        );
    }

    #[test]
    fn lesson_dates_are_china_time() {
        assert_eq!(lesson_date("2026-06-18 18:55:00"), "2026-06-18_18-55");
        assert_eq!(lesson_date("2026-06-18T10:55:00Z"), "2026-06-18_18-55");
        assert_eq!(lesson_date("2026-06-18 08:00"), "2026-06-18_08-00");
        assert_eq!(lesson_date("明天"), "");
    }

    #[test]
    fn allocation_never_reuses_a_taken_name() {
        let directory = tempfile::tempdir().unwrap();
        let layout = file_layout("课", "1", "讲义.pdf");
        let first = allocate(directory.path(), &layout).unwrap();
        let second = allocate(directory.path(), &layout).unwrap();
        assert_eq!(first.file_name().unwrap(), "讲义.pdf");
        assert_eq!(second.file_name().unwrap(), "讲义 (2).pdf");
        assert!(part_path(&first).exists());

        std::fs::write(part_path(&first), b"done").unwrap();
        assert_eq!(finish(&first).unwrap(), first);
        assert_eq!(std::fs::read(&first).unwrap(), b"done");
        // A completed file keeps its name; the next download gets a new one.
        let third = allocate(directory.path(), &layout).unwrap();
        assert_eq!(third.file_name().unwrap(), "讲义 (3).pdf");
        // If the name was taken while downloading, finishing picks another.
        std::fs::write(&third, b"someone else").unwrap();
        std::fs::write(part_path(&third), b"ours").unwrap();
        let finished = finish(&third).unwrap();
        assert_eq!(finished.file_name().unwrap(), "讲义 (4).pdf");
        assert_eq!(std::fs::read(&third).unwrap(), b"someone else");
    }

    #[test]
    fn numbered_names_have_a_base() {
        assert_eq!(base_name("讲义 (3).pdf"), ("讲义.pdf".to_string(), 3));
        assert_eq!(base_name("讲义.pdf"), ("讲义.pdf".to_string(), 1));
        assert_eq!(base_name("README (2)"), ("README".to_string(), 2));
        assert_eq!(base_name("版本 (1).txt"), ("版本 (1).txt".to_string(), 1));
        assert_eq!(base_name("(2).txt"), ("(2).txt".to_string(), 1));
    }
}
