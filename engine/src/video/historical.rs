//! The old player (“课堂视频旧版”): the courses.sjtu.edu.cn LTI 1.1 tool that
//! Canvas still lists next to the new platform. Recordings made before the
//! migration to the new platform (`Config::old_platform_cutoff`) were not
//! carried over and can only be fetched from here.
use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

use super::*;

const PAGE_SIZE: usize = 100;
const ID_PREFIX: &str = "h_";
pub(super) const SOURCE: &str = "historical";

impl VideoService {
    /// The course's recordings on the old player, or None when the course
    /// has no “课堂视频旧版” entry in its Canvas navigation.
    pub(super) async fn historical_course(
        &self,
        jar: Arc<Jar>,
        course_id: &str,
    ) -> AppResult<Option<Vec<Lesson>>> {
        let canvas = Url::parse(&self.config.canvas_origin)?;
        let courses = Url::parse(&self.config.courses_origin)?;
        let client = build_client(
            jar.clone(),
            Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("Too many historical LTI redirects")
                } else if same_origin(attempt.url(), &canvas)
                    || same_origin(attempt.url(), &courses)
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }),
        )?;
        let response = client
            .get(format!("{}/courses/{course_id}", self.config.canvas_origin))
            .send()
            .await?;
        check_status(&response)?;
        let html = response.text().await?;
        let Some(tool_id) = historical_tool_id(&html, &self.config.canvas_origin, course_id)?
        else {
            return Ok(None);
        };
        let response = client
            .get(format!(
                "{}/courses/{course_id}/external_tools/{tool_id}",
                self.config.canvas_origin
            ))
            .send()
            .await?;
        check_status(&response)?;
        let html = response.text().await?;
        let (action, fields) = find_form(&html, |action| action.ends_with("/lti/launch"))?
            .ok_or_else(|| AppError::Upstream("旧版课堂视频没有返回授权表单".into()))?;
        let action = absolute_action(&self.config.courses_origin, &action)?;
        let lti_root = format!("{}/lti", self.config.courses_origin.trim_end_matches('/'));
        validate_video_action(&action, &lti_root)?;
        if action.path() != "/lti/launch" || action.query().is_some() || action.fragment().is_some()
        {
            return Err(AppError::Upstream("旧版视频授权表单指向了未知入口".into()));
        }
        let response = client.post(action).form(&fields).send().await?;
        check_status(&response)?;
        let page_url = response.url().clone();
        validate_video_action(&page_url, &lti_root)?;
        let html = response.text().await?;
        let key = historical_course_key(&html, &page_url, &self.config.courses_origin)?;
        Ok(Some(self.historical_lessons(jar, &key).await?))
    }

    async fn historical_post(
        &self,
        client: &Client,
        path: &str,
        form: &[(&str, String)],
    ) -> AppResult<Value> {
        let response = client
            .post(format!(
                "{}{path}",
                self.config.courses_origin.trim_end_matches('/')
            ))
            .form(form)
            .send()
            .await?;
        check_status(&response)?;
        let payload: Value = response.json().await?;
        historical_body(&payload)?;
        Ok(payload)
    }

    async fn historical_lessons(&self, jar: Arc<Jar>, course_key: &str) -> AppResult<Vec<Lesson>> {
        let client = build_client(jar, Policy::none())?;
        let mut lessons = Vec::new();
        let mut seen = HashSet::new();
        for page_index in 1..=200 {
            let payload = self
                .historical_post(
                    &client,
                    "/lti/vodVideo/findVodVideoList",
                    &[
                        ("canvasCourseId", course_key.to_string()),
                        ("pageIndex", page_index.to_string()),
                        ("pageSize", PAGE_SIZE.to_string()),
                    ],
                )
                .await?;
            let body = historical_body(&payload)?;
            let records = body.get("list").and_then(Value::as_array).ok_or_else(|| {
                AppError::Upstream("旧版视频列表缺少 list，不能判断为没有录像".into())
            })?;
            for record in records {
                let lesson = historical_lesson(record, lessons.len())?;
                if !seen.insert(lesson.video_id.clone()) {
                    return Err(AppError::Upstream(
                        "旧版视频列表重复分页，未返回不完整列表".into(),
                    ));
                }
                lessons.push(lesson);
            }
            if !historical_has_next(body, page_index, seen.len(), records.len())? {
                lessons.sort_by(|a, b| a.begin_time.cmp(&b.begin_time));
                return Ok(lessons);
            }
        }
        Err(AppError::Upstream("旧版视频列表超过分页安全上限".into()))
    }

    pub(super) async fn historical_video_detail(
        &self,
        jar: Arc<Jar>,
        lesson_id: &str,
    ) -> AppResult<VideoDetail> {
        // The caller has already checked membership in this user's course and
        // the current audit/open status. Opaque IDs are decoded only afterwards.
        let id = historical_raw_id(lesson_id)?;
        let client = build_client(jar, Policy::none())?;
        let payload = self
            .historical_post(
                &client,
                "/lti/vodVideo/getVodVideoInfos",
                &[
                    ("id", id),
                    ("playTypeHls", "true".into()),
                    ("isAudit", "true".into()),
                ],
            )
            .await?;
        historical_detail(historical_body(&payload)?)
    }
}

fn check_status(response: &reqwest::Response) -> AppResult<()> {
    if response.status() == StatusCode::SERVICE_UNAVAILABLE {
        // A courses.sjtu.edu.cn outage must not open the independent v.sjtu
        // LTI circuit or make healthy new-platform courses appear unavailable.
        return Err(AppError::upstream_unavailable(
            "旧版课堂视频服务暂时不可用，请稍后重试",
            30,
        ));
    }
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return Err(AppError::VideoUnavailable(
            "旧版视频拒绝访问，请在 Canvas 官网核对当前课程权限".into(),
        ));
    }
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "旧版视频返回 HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

fn historical_tool_id(
    html: &str,
    canvas_origin: &str,
    course_id: &str,
) -> AppResult<Option<String>> {
    let base = Url::parse(canvas_origin)?;
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").expect("static selector");
    let prefix = format!("/courses/{course_id}/external_tools/");
    for link in document.select(&selector) {
        let label = link.text().collect::<String>();
        if !label.contains("课堂视频") || !label.contains("旧版") {
            continue;
        }
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Ok(url) = base.join(href) else { continue };
        if !same_origin(&url, &base) {
            continue;
        }
        let Some(id) = url.path().strip_prefix(&prefix) else {
            continue;
        };
        if validate_canvas_id(id).is_ok() {
            return Ok(Some(id.into()));
        }
    }
    Ok(None)
}

fn historical_course_key(html: &str, page_url: &Url, courses_origin: &str) -> AppResult<String> {
    let document = Html::parse_document(html);
    let base = Url::parse(courses_origin)?;
    let selector = Selector::parse("a[href]").expect("static selector");
    let candidates = std::iter::once(page_url.clone()).chain(
        document
            .select(&selector)
            .filter_map(|link| page_url.join(link.value().attr("href")?).ok()),
    );
    for url in candidates {
        if !same_origin(&url, &base) || url.path() != "/lti/app/lti/vodVideo/playPage" {
            continue;
        }
        // The official page uses the percent-encoded course key verbatim as its
        // form value. query_pairs() would decode it and break the course scope.
        if let Some(key) = url
            .query()
            .into_iter()
            .flat_map(|q| q.split('&'))
            .find_map(|pair| pair.strip_prefix("canvasCourseId="))
            && !key.is_empty()
            && key.len() <= 2048
            && key.is_ascii()
        {
            return Ok(key.into());
        }
    }
    Err(AppError::Upstream(
        "旧版视频授权未返回课程点播入口，请重新进入课程".into(),
    ))
}

fn historical_body(payload: &Value) -> AppResult<&Value> {
    if int(payload, "code") != Some(200) {
        let desc = payload
            .get("desc")
            .and_then(Value::as_str)
            .unwrap_or("返回状态异常");
        return Err(AppError::VideoUnavailable(format!(
            "旧版视频：{}",
            desc.chars().take(300).collect::<String>()
        )));
    }
    payload
        .get("body")
        .filter(|v| v.is_object())
        .ok_or_else(|| AppError::Upstream("旧版视频响应缺少 body".into()))
}

fn int(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

fn historical_has_next(
    body: &Value,
    current: usize,
    count: usize,
    page_len: usize,
) -> AppResult<bool> {
    let page = body
        .get("page")
        .ok_or_else(|| AppError::Upstream("旧版视频缺少分页信息".into()))?;
    let total = int(page, "rowCount")
        .filter(|n| *n >= 0)
        .ok_or_else(|| AppError::Upstream("旧版视频缺少总记录数".into()))? as usize;
    if int(page, "pageIndex") != Some(current as i64)
        || count > total
        || (count < total && page_len == 0)
    {
        return Err(AppError::Upstream(
            "旧版视频分页信息不一致，未返回不完整列表".into(),
        ));
    }
    Ok(count < total)
}

/// One record of the old player's list. Its ids are opaque base64 strings
/// with characters the apps and file names must not see, so they are carried
/// URL-safe with an `h_` prefix.
fn historical_lesson(record: &Value, index: usize) -> AppResult<Lesson> {
    let raw_id = value_id(record, "videoId")
        .ok_or_else(|| AppError::Upstream("旧版视频记录缺少 videoId".into()))?;
    validate_historical_raw_id(&raw_id)?;
    let video_id = format!("{ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw_id.as_bytes()));
    validate_resource_id(&video_id)?;
    let string = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| record.get(*key).and_then(Value::as_str))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let audit_status = int(record, "videAuditStatus").unwrap_or_default();
    let title = string(&["courseName", "videoName", "title"]);
    Ok(Lesson {
        video_id,
        title: if title.is_empty() {
            format!("第 {:02} 讲", index + 1)
        } else {
            title
        },
        begin_time: string(&["courseBeginTime", "beginTime"]),
        end_time: string(&["courseEndTime", "endTime"]),
        classroom: string(&["classroomName", "classroom", "roomName"]),
        audit_status,
        available: audit_status == 3,
        source: SOURCE.into(),
    })
}

fn validate_historical_raw_id(id: &str) -> AppResult<()> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
    {
        return Err(AppError::BadRequest("旧版录像 ID 无效".into()));
    }
    Ok(())
}

fn historical_raw_id(id: &str) -> AppResult<String> {
    validate_resource_id(id)?;
    let encoded = id
        .strip_prefix(ID_PREFIX)
        .ok_or_else(|| AppError::BadRequest("不是旧版录像 ID".into()))?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|v| String::from_utf8(v).ok())
        .ok_or_else(|| AppError::BadRequest("旧版录像 ID 编码无效".into()))?;
    validate_historical_raw_id(&decoded)?;
    Ok(decoded)
}

/// True for ids `historical_lesson` produced.
pub(super) fn is_historical_id(id: &str) -> bool {
    id.starts_with(ID_PREFIX)
}

fn historical_detail(body: &Value) -> AppResult<VideoDetail> {
    let views = body
        .get("videoPlayResponseVoList")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Upstream("旧版视频详情缺少分轨列表".into()))?;
    let tracks = views
        .iter()
        .filter_map(|view| {
            // The best quality the player offers, like its own quality menu.
            let url = [
                "rtmpUrlHdv",
                "rtmpUrlDistinct",
                "rtmpUrlHd",
                "rtmpUrlDefault",
                "rtmpUrlFluency",
                "rtmpUrl",
            ]
            .iter()
            .find_map(|key| {
                view.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })?;
            Some(VideoTrack {
                id: value_id(view, "id")?,
                view: int(view, "cdviViewNum")?,
                url: url.to_string(),
            })
        })
        .collect::<Vec<_>>();
    if tracks.is_empty() {
        return Err(AppError::VideoUnavailable(
            "旧版视频没有返回可用分轨".into(),
        ));
    }
    Ok(VideoDetail { tracks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::tests::lti_test_config;
    use serde_json::json;

    #[test]
    fn discovers_only_current_courses_old_tool() {
        let html = r#"
          <a href="/courses/42/external_tools/8329">课堂视频new</a>
          <a href="https://evil.example/courses/42/external_tools/1">课堂视频旧版</a>
          <a href="/courses/43/external_tools/2">课堂视频旧版</a>
          <a href="/courses/42/external_tools/9487">课堂视频旧版</a>"#;
        assert_eq!(
            historical_tool_id(html, "https://oc.sjtu.edu.cn", "42")
                .unwrap()
                .as_deref(),
            Some("9487")
        );
        assert!(
            historical_tool_id(
                "<a href='/courses/42/external_tools/8329'>课堂视频new</a>",
                "https://oc.sjtu.edu.cn",
                "42"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn scope_keeps_official_percent_encoding() {
        let page =
            Url::parse("https://courses.sjtu.edu.cn/lti/app/lti/liveVideo/index.d2j").unwrap();
        let html = r#"<a href="https://evil.example/lti/app/lti/vodVideo/playPage?canvasCourseId=wrong">x</a>
          <a href="/lti/app/lti/vodVideo/playPage?canvasCourseId=opaque%2Bscope%3D&amp;other=1">课程点播</a>"#;
        assert_eq!(
            historical_course_key(html, &page, "https://courses.sjtu.edu.cn").unwrap(),
            "opaque%2Bscope%3D"
        );
        assert!(
            historical_course_key(
                "<a href='/lti/other?canvasCourseId=bad'>x</a>",
                &page,
                "https://courses.sjtu.edu.cn"
            )
            .is_err()
        );
    }

    #[test]
    fn encrypted_ids_are_transport_safe_and_round_trip() {
        let raw = "VSLRUCM+nBaq/1k95pBoBCQ==";
        let lesson = historical_lesson(
            &json!({"videoId":raw,"videAuditStatus":3,"videoName":"测试第1讲","courseBeginTime":"2026-03-02 08:00:00"}),
            0,
        )
        .unwrap();
        validate_resource_id(&lesson.video_id).unwrap();
        assert!(is_historical_id(&lesson.video_id));
        assert_eq!(historical_raw_id(&lesson.video_id).unwrap(), raw);
        assert!(lesson.available);
        assert_eq!(lesson.source, SOURCE);
        assert_eq!(lesson.title, "测试第1讲");
        assert_eq!(lesson.begin_time, "2026-03-02 08:00:00");
        assert!(
            !historical_lesson(&json!({"videoId":"a==","videAuditStatus":1}), 0)
                .unwrap()
                .available
        );
        for id in [
            "h_%%%",
            "1234",
            "h_",
            "h_Li4vLi4vZm9v",
            "h_aHR0cHM6Ly9ldmlsLmV4YW1wbGU",
        ] {
            assert!(historical_raw_id(id).is_err());
        }
    }

    #[test]
    fn errors_and_incomplete_pages_are_not_empty_successes() {
        assert!(historical_body(&json!({"code":403,"body":{"list":[]}})).is_err());
        assert!(historical_body(&json!({"code":200,"body":null})).is_err());
        assert!(
            historical_has_next(&json!({"page":{"pageIndex":1,"rowCount":150}}), 1, 100, 100)
                .unwrap()
        );
        assert!(
            !historical_has_next(&json!({"page":{"pageIndex":2,"rowCount":150}}), 2, 150, 50)
                .unwrap()
        );
        assert!(
            historical_has_next(&json!({"page":{"pageIndex":2,"rowCount":150}}), 2, 100, 0)
                .is_err()
        );
        assert!(
            historical_has_next(&json!({"page":{"pageIndex":1,"rowCount":150}}), 2, 150, 50)
                .is_err()
        );
    }

    #[test]
    fn historical_media_quality_fields_match_live_player() {
        let detail = historical_detail(&json!({"videoPlayResponseVoList":[
          {"id":123,"cdviViewNum":0,"rtmpUrlHdv":"https://media.example/hd.mp4","rtmpUrlDefault":"https://media.example/default.mp4"},
          {"id":124,"cdviViewNum":3,"rtmpUrlHdv":"","rtmpUrlDistinct":"https://media.example/ppt.mp4"},
          {"id":125,"cdviViewNum":4}
        ]}))
        .unwrap();
        assert_eq!(detail.tracks.len(), 2);
        assert_eq!(detail.tracks[0].url, "https://media.example/hd.mp4");
        assert_eq!(detail.tracks[0].view, 0);
        assert_eq!(detail.tracks[1].url, "https://media.example/ppt.mp4");
        assert_eq!(detail.tracks[1].view, 3);
        assert!(historical_detail(&json!({"videoPlayResponseVoList":[]})).is_err());
    }

    #[tokio::test]
    async fn historical_listing_posts_scope_and_reads_every_page() {
        use axum::{Form, Json, Router, routing::post};
        let app = Router::new().route(
            "/lti/vodVideo/findVodVideoList",
            post(|Form(form): Form<HashMap<String, String>>| async move {
                assert_eq!(form["canvasCourseId"], "opaque%2Bscope");
                assert_eq!(form["pageSize"], "100");
                let page: usize = form["pageIndex"].parse().unwrap();
                Json(json!({"code":200,"body":{"page":{"pageIndex":page,"rowCount":2},"list":[{"videoId":format!("encrypted+{page}=="),"videAuditStatus":3}]}}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = lti_test_config();
        config.courses_origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let service = VideoService::new(Arc::new(config));
        let result = service
            .historical_lessons(Arc::new(Jar::default()), "opaque%2Bscope")
            .await;
        task.abort();
        let lessons = result.unwrap();
        assert_eq!(lessons.len(), 2);
        assert_eq!(
            historical_raw_id(&lessons[1].video_id).unwrap(),
            "encrypted+2=="
        );
        assert!(lessons.iter().all(|lesson| lesson.source == SOURCE));
    }
}
