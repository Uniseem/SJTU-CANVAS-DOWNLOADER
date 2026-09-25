//! A demo school for UI tests and screenshots, available only in debug builds
//! with SJTU_CANVAS_TEST_MODE=1 and SJTU_CANVAS_FAKE_SCHOOL=1. The login
//! completes by itself, courses and lessons are fixed, and media comes from a
//! local server (engine/tests/mock_media.py) at SJTU_CANVAS_FAKE_MEDIA.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use reqwest::cookie::Jar;
use tokio::sync::mpsc;
use url::Url;

use crate::{
    auth::{AuthManager, LoginAttempt, LoginCommand, LoginEvent},
    config::Config,
    downloads::{DownloadRow, Media},
    error::{AppError, AppResult},
    models::{
        CanvasFile, CanvasProfile, Course, Lesson, LessonSizes, VideoSizeStatus, VideoTrackSize,
    },
};

pub fn profile() -> CanvasProfile {
    CanvasProfile {
        id: "20260001".into(),
        name: "演示同学".into(),
        short_name: "演示同学".into(),
        avatar_url: None,
    }
}

pub async fn run_login(
    manager: &Arc<AuthManager>,
    attempt: &Arc<LoginAttempt>,
    commands: &mut mpsc::Receiver<LoginCommand>,
) {
    let delay = manager.config.fake_login_delay;
    let mut generation = 1u32;
    loop {
        let mut event = LoginEvent::new(
            &attempt.id,
            "waiting",
            "演示模式：无需扫码，稍后自动登录",
            generation,
        );
        event.qr_png = Some(STANDARD.encode(qr_png(generation)));
        event.expires_at = Some(
            (Utc::now() + chrono::Duration::seconds(58)).to_rfc3339_opts(SecondsFormat::Secs, true),
        );
        attempt.publish(event).await;
        tokio::select! {
            command = commands.recv() => match command {
                Some(LoginCommand::Refresh) => {
                    generation += 1;
                    continue;
                }
                _ => {
                    attempt.publish(LoginEvent::new(&attempt.id, "cancelled", "已取消扫码登录", generation)).await;
                    return;
                }
            },
            _ = tokio::time::sleep(delay) => break,
        }
    }
    attempt
        .publish(LoginEvent::new(
            &attempt.id,
            "authorizing",
            "扫码成功，正在验证 Canvas 身份…",
            generation,
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let profile = profile();
    let message = format!("已登录 {} 的 Canvas", profile.name);
    match manager
        .finish_login(Arc::new(Jar::default()), profile)
        .await
    {
        Ok(()) => {
            attempt
                .publish(LoginEvent::new(
                    &attempt.id,
                    "authorized",
                    message,
                    generation,
                ))
                .await
        }
        Err(error) => {
            attempt
                .publish(LoginEvent::new(&attempt.id, "error", error, generation))
                .await
        }
    }
}

const COURSES: [(&str, &str, &str, &str, &str, &str); 6] = [
    (
        "87954",
        "机器学习与数据挖掘（2026 秋）",
        "CS3611",
        "2026 秋季学期",
        "陈老师",
        "active",
    ),
    (
        "88148",
        "无线通信原理",
        "EE3402",
        "2026 秋季学期",
        "何老师",
        "active",
    ),
    (
        "88311",
        "常微分方程",
        "MATH2201",
        "2026 秋季学期",
        "李老师、王老师",
        "active",
    ),
    (
        "88420",
        "中国近现代史纲要",
        "MARX1203",
        "2026 秋季学期",
        "刘老师",
        "active",
    ),
    (
        "76231",
        "大学物理（荣誉）",
        "PHYS1201",
        "2026 春季学期",
        "周老师",
        "completed",
    ),
    (
        "74002",
        "程序设计思想与方法（C++）",
        "CS1501",
        "2025 秋季学期",
        "吴老师",
        "completed",
    ),
];

const TOPICS: [&str; 10] = [
    "课程导论",
    "线性模型",
    "反向传播",
    "优化方法",
    "卷积网络",
    "循环网络",
    "注意力机制",
    "生成模型",
    "强化学习",
    "课程总结",
];

pub fn courses() -> Vec<Course> {
    COURSES
        .iter()
        .map(|(id, name, code, term, teacher, state)| Course {
            id: (*id).into(),
            name: (*name).into(),
            course_code: (*code).into(),
            start_at: None,
            end_at: None,
            term: Some((*term).into()),
            teacher: Some((*teacher).into()),
            enrollment_state: (*state).into(),
        })
        .collect()
}

fn known_course(course_id: &str) -> AppResult<()> {
    if COURSES.iter().any(|course| course.0 == course_id) {
        Ok(())
    } else {
        Err(AppError::NotFound("课程不存在".into()))
    }
}

pub fn lessons(course_id: &str) -> AppResult<Vec<Lesson>> {
    known_course(course_id)?;
    let machine_learning = course_id == "87954";
    Ok((1..=10)
        .map(|index| {
            let topic = if machine_learning {
                format!("第 {index:02} 讲 · {}", TOPICS[index - 1])
            } else {
                format!("第 {index:02} 讲")
            };
            Lesson {
                video_id: format!("demo-{course_id}-{index:02}"),
                title: topic,
                begin_time: format!("2026-09-{index:02} 08:00:00"),
                end_time: format!("2026-09-{index:02} 09:40:00"),
                classroom: if index % 2 == 0 {
                    "东中院 4-202"
                } else {
                    "东上院 101"
                }
                .into(),
                audit_status: if index == 10 { 1 } else { 3 },
                // The newest lesson is still being processed, like on the real platform.
                available: index != 10,
            }
        })
        .collect())
}

pub fn files(course_id: &str) -> AppResult<Vec<CanvasFile>> {
    known_course(course_id)?;
    let file = |id: &str, name: &str, size: u64, kind: &str, updated: &str| CanvasFile {
        id: format!("{course_id}{id}"),
        display_name: name.into(),
        filename: name.into(),
        size,
        content_type: Some(kind.into()),
        updated_at: Some(updated.into()),
        url: None,
    };
    Ok(vec![
        file(
            "01",
            "课程大纲.pdf",
            482_133,
            "application/pdf",
            "2026-08-30T02:10:00Z",
        ),
        file(
            "02",
            "第 01 讲 课件.pdf",
            2_431_616,
            "application/pdf",
            "2026-09-01T09:50:00Z",
        ),
        file(
            "03",
            "第 02 讲 课件.pdf",
            3_874_221,
            "application/pdf",
            "2026-09-02T09:50:00Z",
        ),
        file(
            "04",
            "作业 1 说明.docx",
            96_512,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "2026-09-03T12:00:00Z",
        ),
        file(
            "05",
            "实验数据集.zip",
            18_761_233,
            "application/zip",
            "2026-09-05T06:10:00Z",
        ),
    ])
}

/// Sizes of the demo recordings: 20–45 MB, no composite view.
fn track_size(lesson_id: &str, track: &str) -> Option<u64> {
    if track == "composite" {
        return None;
    }
    let hash = crate::downloads::naming::short_id(&format!("{lesson_id}/{track}"));
    let seed = u64::from_str_radix(&hash, 16).unwrap_or(0);
    Some(20_000_000 + seed % 25_000_000)
}

pub fn sizes(course_id: &str, lesson_id: &str, tracks: &[String]) -> AppResult<LessonSizes> {
    let lesson = lessons(course_id)?
        .into_iter()
        .find(|lesson| lesson.video_id == lesson_id && lesson.available)
        .ok_or_else(|| AppError::NotFound("该讲次不存在、未开放，或不属于当前课程".into()))?;
    let tracks = tracks
        .iter()
        .map(|track| {
            let size = track_size(&lesson.video_id, track);
            (
                track.clone(),
                VideoTrackSize {
                    status: if size.is_some() {
                        VideoSizeStatus::Ready
                    } else {
                        VideoSizeStatus::Missing
                    },
                    size,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(LessonSizes {
        video_id: lesson.video_id,
        tracks,
    })
}

pub fn resolve_download(config: &Config, row: &DownloadRow) -> AppResult<Media> {
    let base = config.fake_media.as_deref().ok_or_else(|| {
        AppError::VideoUnavailable("演示模式没有配置媒体服务（SJTU_CANVAS_FAKE_MEDIA）".into())
    })?;
    let (size, filename) = match row.kind.as_str() {
        "video" => {
            let track = row.track.as_deref().unwrap_or("teacher");
            let size = track_size(&row.resource_id, track)
                .ok_or_else(|| AppError::NotFound("当前讲次没有所选画面".into()))?;
            (size, format!("{}.mp4", row.resource_id))
        }
        _ => {
            let file = files(&row.course_id)?
                .into_iter()
                .find(|file| file.id == row.resource_id)
                .ok_or_else(|| AppError::NotFound("文件不存在".into()))?;
            (file.size, file.filename)
        }
    };
    let mut url = Url::parse(&format!("{base}/media/{}", row.id))?;
    url.query_pairs_mut().append_pair("size", &size.to_string());
    if let Ok(rate) = std::env::var("SJTU_CANVAS_FAKE_RATE") {
        url.query_pairs_mut().append_pair("rate", rate.trim());
    }
    Ok(Media {
        url,
        headers: Vec::new(),
        filename,
        size: Some(size),
    })
}

/// A QR-code-like placeholder (not a scannable code) as a grayscale PNG.
fn qr_png(seed: u32) -> Vec<u8> {
    const MODULES: usize = 25;
    const SCALE: usize = 8;
    const MARGIN: usize = 4;
    let side = (MODULES + 2 * MARGIN) * SCALE;
    let mut state = 0x9e37_79b9_u32 ^ seed.wrapping_mul(2_654_435_761);
    let mut dark = [[false; MODULES]; MODULES];
    for row in dark.iter_mut() {
        for cell in row.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *cell = state % 5 < 2;
        }
    }
    for (top, left) in [(0, 0), (0, MODULES - 7), (MODULES - 7, 0)] {
        for y in 0..7 {
            for x in 0..7 {
                let ring = y == 0 || y == 6 || x == 0 || x == 6;
                let core = (2..=4).contains(&y) && (2..=4).contains(&x);
                dark[top + y][left + x] = ring || core;
            }
        }
    }
    let mut pixels = vec![255u8; side * side];
    for (y, row) in dark.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if *cell {
                for dy in 0..SCALE {
                    let start = ((y + MARGIN) * SCALE + dy) * side + (x + MARGIN) * SCALE;
                    pixels[start..start + SCALE].fill(0);
                }
            }
        }
    }
    png_gray(side as u32, side as u32, &pixels)
}

/// A minimal PNG encoder: 8-bit grayscale, stored (uncompressed) deflate.
fn png_gray(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize + 1) * height as usize);
    for row in pixels.chunks(width as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).collect::<Vec<_>>();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let length = block.len() as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 0, 0, 0, 0]);
    for (kind, data) in [
        (&b"IHDR"[..], &header[..]),
        (b"IDAT", &zlib),
        (b"IEND", &[]),
    ] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_qr_is_a_valid_png() {
        let png = qr_png(1);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 264);
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn demo_data_is_consistent() {
        let demo = lessons("87954").unwrap();
        assert_eq!(demo.len(), 10);
        assert!(!demo[9].available);
        let sizes = sizes(
            "87954",
            &demo[0].video_id,
            &["slides".into(), "composite".into()],
        )
        .unwrap();
        assert_eq!(sizes.tracks["slides"].status, VideoSizeStatus::Ready);
        assert_eq!(sizes.tracks["composite"].status, VideoSizeStatus::Missing);
        assert!(lessons("1").is_err());
    }
}
