//! Small, authenticated metadata probes. Never consume a media response body.
use super::*;
use crate::models::{LessonSizes, VideoSizeStatus, VideoTrackSize};
use reqwest::{Method, header::HeaderMap};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};
use tokio::sync::Semaphore;

type SizeKey = (String, String, String, String);
const CACHE_LIMIT: usize = 4096;
const READY_TTL: Duration = Duration::from_secs(15 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(30);

struct CachedSize {
    value: VideoTrackSize,
    expires: Instant,
}
pub(super) struct SizeCache {
    entries: DashMap<SizeKey, CachedSize>,
    // Fixed-size striped locks bound memory while coalescing concurrent reads.
    gates: [Mutex<()>; 64],
    slots: Semaphore,
}
impl Default for SizeCache {
    fn default() -> Self {
        Self {
            entries: DashMap::new(),
            gates: std::array::from_fn(|_| Mutex::new(())),
            slots: Semaphore::new(4),
        }
    }
}
impl SizeCache {
    pub(super) fn clear_owner(&self, owner: &str) {
        self.entries
            .retain(|(session, _, _, _), _| session != owner);
    }
    fn get(&self, key: &SizeKey) -> Option<VideoTrackSize> {
        self.entries
            .get(key)
            .filter(|entry| entry.expires > Instant::now())
            .map(|entry| entry.value.clone())
    }
    fn put(&self, key: SizeKey, value: VideoTrackSize) {
        if self.entries.len() >= CACHE_LIMIT {
            self.entries
                .retain(|_, entry| entry.expires > Instant::now());
        }
        if self.entries.len() < CACHE_LIMIT || self.entries.contains_key(&key) {
            let ttl = if value.status == VideoSizeStatus::Ready {
                READY_TTL
            } else {
                FAILURE_TTL
            };
            self.entries.insert(
                key,
                CachedSize {
                    value,
                    expires: Instant::now() + ttl,
                },
            );
        }
    }
}

pub(super) fn parse_tracks(value: &str) -> AppResult<Vec<String>> {
    let mut tracks = Vec::new();
    if value.len() > 64 {
        return Err(AppError::BadRequest("录像分轨参数过长".into()));
    }
    for kind in value.split(',') {
        if !["teacher", "slides", "composite"].contains(&kind) {
            return Err(AppError::BadRequest("未知的视频分轨".into()));
        }
        if !tracks.iter().any(|track| track == kind) {
            tracks.push(kind.to_owned());
        }
    }
    Ok(tracks)
}

impl VideoService {
    pub fn cached_track_size(
        &self,
        owner: &str,
        course: &str,
        lesson: &str,
        track: &str,
    ) -> Option<u64> {
        self.sizes
            .get(&(owner.into(), course.into(), lesson.into(), track.into()))
            .and_then(|entry| entry.size)
    }

    pub async fn lesson_sizes(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        course_id: &str,
        lesson_id: &str,
        tracks: &str,
        refresh: bool,
    ) -> AppResult<LessonSizes> {
        validate_canvas_id(course_id)?;
        validate_resource_id(lesson_id)?;
        let tracks = parse_tracks(tracks)?;
        source_with_timeout(Duration::from_secs(25), "录像大小查询", async {
            let cached = self.course(owner, jar.clone(), course_id, false).await?;
            let lesson = cached
                .lessons
                .iter()
                .find(|lesson| lesson.video_id == lesson_id && lesson.available)
                .ok_or_else(|| {
                    AppError::NotFound("该讲次不存在、未开放，或不属于当前课程".into())
                })?;
            let mut hasher = DefaultHasher::new();
            (owner, course_id, lesson_id).hash(&mut hasher);
            let _gate = self.sizes.gates[hasher.finish() as usize % self.sizes.gates.len()]
                .lock()
                .await;
            let key = |kind: &str| {
                (
                    owner.to_owned(),
                    course_id.to_owned(),
                    lesson_id.to_owned(),
                    kind.to_owned(),
                )
            };
            let mut values = BTreeMap::new();
            let mut pending = Vec::new();
            for kind in tracks {
                let cached_size = self.sizes.get(&key(&kind));
                // A manual retry bypasses negative cache entries, not known sizes.
                if let Some(value) =
                    cached_size.filter(|value| !refresh || value.status == VideoSizeStatus::Ready)
                {
                    values.insert(kind, value);
                } else {
                    pending.push(kind);
                }
            }
            if !pending.is_empty() {
                let _slot = self
                    .sizes
                    .slots
                    .acquire()
                    .await
                    .map_err(AppError::internal)?;
                let detail = source_with_timeout(
                    Duration::from_secs(10),
                    "录像详情",
                    self.cached_video_detail(
                        owner,
                        course_id,
                        jar.clone(),
                        &cached.session,
                        lesson_id,
                    ),
                )
                .await;
                let client = metadata_client(jar.clone())?;
                for kind in pending {
                    let value = match &detail {
                        Ok(detail) => match detail
                            .video_play_response_vo_list
                            .iter()
                            .find(|track| track.cdvi_view_num == track_code(&kind).unwrap_or(-1))
                        {
                            Some(track) => {
                                // Use the same quality/URL selection as the real download.
                                let probe = async {
                                    let resolved = self
                                        .resolve_video_url(
                                            jar.clone(),
                                            &cached.session,
                                            lesson,
                                            track,
                                        )
                                        .await?;
                                    probe_size(&client, &resolved).await
                                };
                                let size =
                                    source_with_timeout(Duration::from_secs(6), "视频大小", probe)
                                        .await
                                        .ok()
                                        .flatten();
                                VideoTrackSize {
                                    status: if size.is_some() {
                                        VideoSizeStatus::Ready
                                    } else {
                                        VideoSizeStatus::Unavailable
                                    },
                                    size,
                                }
                            }
                            None => VideoTrackSize {
                                status: VideoSizeStatus::Missing,
                                size: None,
                            },
                        },
                        Err(_) => VideoTrackSize {
                            status: VideoSizeStatus::Unavailable,
                            size: None,
                        },
                    };
                    self.sizes.put(key(&kind), value.clone());
                    values.insert(kind, value);
                }
            }
            Ok(LessonSizes {
                video_id: lesson_id.into(),
                tracks: values,
            })
        })
        .await
    }
}

fn metadata_client(jar: Arc<Jar>) -> AppResult<Client> {
    crate::http::apply(Client::builder())
        .http1_only()
        .cookie_provider(jar)
        .redirect(Policy::none())
        .user_agent(crate::http::user_agent())
        .gzip(false)
        .brotli(false)
        .deflate(false)
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(AppError::internal)
}

async fn probe_size(client: &Client, media: &ResolvedVideo) -> AppResult<Option<u64>> {
    // HEAD normally returns only metadata; fallback asks for a single byte.
    // Dropping the response immediately also stops servers which ignore Range.
    for method in [Method::HEAD, Method::GET] {
        if let Ok(response) = probe_response(client, media, method).await
            && let Some(size) = size_from_headers(response.status(), response.headers())
        {
            return Ok(Some(size));
        }
    }
    Ok(None)
}

async fn probe_response(
    client: &Client,
    media: &ResolvedVideo,
    method: Method,
) -> AppResult<reqwest::Response> {
    let mut url = media.upstream_url.clone();
    let mut credentials_allowed = true;
    for _ in 0..5 {
        validate_generated_url(&url)?;
        let mut request = client
            .request(method.clone(), url.clone())
            .header(header::ACCEPT_ENCODING, "identity");
        for (name, value) in &media.headers {
            if name.eq_ignore_ascii_case("referer") || credentials_allowed {
                request = request.header(name, value);
            }
        }
        if method == Method::GET {
            request = request.header(header::RANGE, "bytes=0-0");
        }
        let response = request.send().await?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|header| header.to_str().ok())
            .ok_or_else(|| AppError::Upstream("视频大小查询跳转缺少地址".into()))?;
        let next = url.join(location)?;
        credentials_allowed &= same_origin(&next, &media.upstream_url);
        url = next;
    }
    Err(AppError::Upstream("视频大小查询跳转过多".into()))
}

fn size_from_headers(status: StatusCode, headers: &HeaderMap) -> Option<u64> {
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if text(header::CONTENT_ENCODING).is_some_and(|value| !value.eq_ignore_ascii_case("identity")) {
        return None;
    }
    if let Some(kind) = text(header::CONTENT_TYPE) {
        let kind = kind.split(';').next()?.trim().to_ascii_lowercase();
        if !(kind.starts_with("video/")
            || [
                "application/octet-stream",
                "binary/octet-stream",
                "application/mp4",
            ]
            .contains(&kind.as_str()))
        {
            return None;
        }
    }
    let size = if status == StatusCode::PARTIAL_CONTENT {
        let (range, total) = text(header::CONTENT_RANGE)?
            .strip_prefix("bytes ")?
            .split_once('/')?;
        let (start, end) = range.split_once('-')?;
        let (start, end, total) = (
            start.parse::<u64>().ok()?,
            end.parse::<u64>().ok()?,
            total.parse::<u64>().ok()?,
        );
        if start != 0 || start > end || end >= total {
            return None;
        }
        total
    } else if status == StatusCode::OK {
        text(header::CONTENT_LENGTH)?.parse().ok()?
    } else {
        return None;
    };
    // Keep all values exact in the JSON/JavaScript client; zero-length isn't video.
    (size > 0 && size <= 9_007_199_254_740_991).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::tests::lti_test_config;

    fn headers(values: &[(&str, &str)]) -> HeaderMap {
        values
            .iter()
            .map(|(key, value)| {
                (
                    reqwest::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                    value.parse().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn reads_complete_size_not_the_one_byte_range_length() {
        let range = headers(&[
            ("content-length", "1"),
            ("content-range", "bytes 0-0/734003200"),
            ("content-type", "video/mp4"),
        ]);
        assert_eq!(
            size_from_headers(StatusCode::PARTIAL_CONTENT, &range),
            Some(734003200)
        );
        assert_eq!(
            size_from_headers(StatusCode::OK, &headers(&[("content-length", "734003200")])),
            Some(734003200)
        );
        assert_eq!(
            size_from_headers(
                StatusCode::PARTIAL_CONTENT,
                &headers(&[("content-length", "1")])
            ),
            None
        );
    }

    #[test]
    fn rejects_error_pages_encoding_and_invalid_lengths() {
        for values in [
            vec![
                ("content-length", "1234"),
                ("content-type", "text/html; charset=utf-8"),
            ],
            vec![
                ("content-length", "1234"),
                ("content-type", "application/json"),
            ],
            vec![("content-length", "1234"), ("content-encoding", "gzip")],
            vec![("content-length", "0")],
            vec![("content-length", "-1")],
            vec![("content-length", "9007199254740992")],
            vec![],
        ] {
            assert_eq!(size_from_headers(StatusCode::OK, &headers(&values)), None);
        }
        for status in [
            StatusCode::FORBIDDEN,
            StatusCode::BAD_GATEWAY,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::FOUND,
        ] {
            assert_eq!(
                size_from_headers(status, &headers(&[("content-length", "1234")])),
                None
            );
        }
        for value in [
            "bytes 0-0/*",
            "bytes 0-1/1",
            "bytes 5-1/100",
            "bytes */100",
            "items 0-0/100",
        ] {
            assert_eq!(
                size_from_headers(
                    StatusCode::PARTIAL_CONTENT,
                    &headers(&[("content-range", value)])
                ),
                None
            );
        }
    }

    #[test]
    fn requested_tracks_are_bounded_canonical_and_deduplicated() {
        assert_eq!(
            parse_tracks("slides,teacher,slides").unwrap(),
            vec!["slides", "teacher"]
        );
        for value in ["", "ppt", "teacher,", "../../url", "slides,unknown"] {
            assert!(parse_tracks(value).is_err());
        }
    }

    #[test]
    fn cache_is_scoped_expires_and_is_cleared_on_logout() {
        let cache = SizeCache::default();
        let key: SizeKey = (
            "owner".into(),
            "42".into(),
            "lesson".into(),
            "teacher".into(),
        );
        cache.put(
            key.clone(),
            VideoTrackSize {
                status: VideoSizeStatus::Ready,
                size: Some(123),
            },
        );
        assert_eq!(cache.get(&key).unwrap().size, Some(123));
        assert!(
            cache
                .get(&("other".into(), key.1.clone(), key.2.clone(), key.3.clone()))
                .is_none()
        );
        cache.entries.get_mut(&key).unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(cache.get(&key).is_none());
        cache.put(
            key.clone(),
            VideoTrackSize {
                status: VideoSizeStatus::Unavailable,
                size: None,
            },
        );
        assert!(cache.entries.get(&key).unwrap().expires <= Instant::now() + FAILURE_TTL);
        cache.clear_owner("owner");
        assert!(cache.entries.is_empty());
    }

    #[tokio::test]
    async fn warm_sizes_require_course_membership_and_missing_views_are_not_zero_bytes() {
        let service = VideoService::new(Arc::new(lti_test_config()));
        let lesson = Lesson {
            video_id: "lesson".into(),
            title: "test".into(),
            begin_time: String::new(),
            end_time: String::new(),
            classroom: String::new(),
            audit_status: 3,
            available: true,
            source: Some("historical".into()),
        };
        service.cache.insert(
            ("owner".into(), "42".into()),
            CachedCourse {
                session: VideoSession {
                    token: String::new(),
                    canvas_course_id: "42".into(),
                    protocol: VideoProtocol::Historical,
                },
                lessons: vec![lesson],
                expires_at: Instant::now() + READY_TTL,
            },
        );
        service.sizes.put(
            (
                "owner".into(),
                "42".into(),
                "lesson".into(),
                "teacher".into(),
            ),
            VideoTrackSize {
                status: VideoSizeStatus::Ready,
                size: Some(1234),
            },
        );
        service.detail_cache.insert(
            ("owner".into(), "42".into(), "lesson".into()),
            CachedDetail {
                detail: VideoDetail {
                    video_play_response_vo_list: vec![],
                },
                expires_at: Instant::now() + READY_TTL,
            },
        );
        let result = service
            .lesson_sizes(
                "owner",
                Arc::new(Jar::default()),
                "42",
                "lesson",
                "teacher,slides",
                false,
            )
            .await
            .unwrap();
        assert_eq!(result.tracks["teacher"].size, Some(1234));
        assert_eq!(result.tracks["slides"].status, VideoSizeStatus::Missing);
        assert_eq!(result.tracks["slides"].size, None);
        service
            .cache
            .get_mut(&("owner".into(), "42".into()))
            .unwrap()
            .lessons[0]
            .available = false;
        assert!(matches!(
            service
                .lesson_sizes(
                    "owner",
                    Arc::new(Jar::default()),
                    "42",
                    "lesson",
                    "teacher",
                    false
                )
                .await,
            Err(AppError::NotFound(_))
        ));
    }
}
