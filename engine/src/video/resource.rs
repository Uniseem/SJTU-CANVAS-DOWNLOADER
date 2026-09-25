//! SJTU resource-management video API (August 2026 migration).
//! Endpoints and field mappings follow the school's public player, not the
//! retired canvas-sjtu API. JWTs stay server-side and are never download URLs.
use std::collections::HashSet;

use serde_json::Value;

use super::*;

pub(super) const INITIATION_PATH: &str = "/lti/canvas/oidc/login-initiation/canvas-record";
pub(super) const SOURCE: &str = "resource";
const LAUNCH_PATH: &str = "/lti/canvas/launch/canvas-record";
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 200;

impl VideoService {
    pub(super) async fn authorize_resource_video(
        &self,
        client: &Client,
        no_redirect: &Client,
        action: &str,
        fields: &HashMap<String, String>,
    ) -> AppResult<VideoSession> {
        let initiation_url = absolute_action(&self.config.video_lti_adapter, action)?;
        validate_adapter_action(
            &initiation_url,
            &self.config.video_lti_adapter,
            INITIATION_PATH,
        )?;
        let response = self.initiate_lti(client, &initiation_url, fields).await?;
        let LtiLaunch { url, fields } =
            resolve_lti_launch(self, client, &self.config, response).await?;
        validate_adapter_action(&url, &self.config.video_lti_adapter, LAUNCH_PATH)?;
        let response = self
            .classify_video_response(no_redirect.post(url).form(&fields).send().await?)
            .await?;
        let token = resource_token_from_response(response, &self.config).await?;
        let payload = self
            .resource_get(no_redirect, &token, "/lms/launch-context", &[])
            .await?;
        let class_id = teaching_class_id(&payload)?;
        Ok(VideoSession {
            token,
            teaching_class_id: class_id,
        })
    }

    async fn resource_get(
        &self,
        client: &Client,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> AppResult<Value> {
        let url = format!(
            "{}{path}",
            self.config.resource_video_api.trim_end_matches('/')
        );
        let response = client
            .get(url)
            .header("jwt-token", token)
            .query(query)
            .send()
            .await?;
        let response = self.classify_video_response(response).await?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(AppError::VideoUnavailable(
                "新视频平台拒绝了访问，请从 Canvas 官网核对课程权限后重试".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "新视频平台返回 HTTP {}",
                response.status()
            )));
        }
        let payload: Value = response.json().await?;
        resource_data(&payload)?;
        Ok(payload)
    }

    pub(super) async fn resource_lessons(
        &self,
        jar: Arc<Jar>,
        session: &VideoSession,
    ) -> AppResult<Vec<Lesson>> {
        // JWT headers must never be forwarded through an upstream redirect.
        let client = build_client(jar, Policy::none())?;
        let mut lessons = Vec::new();
        let mut seen = HashSet::new();
        for page in 1..=MAX_PAGES {
            let payload = self
                .resource_get(
                    &client,
                    &session.token,
                    "/v1/subject_vod_list_new",
                    &[
                        ("teclIds", session.teaching_class_id.clone()),
                        ("page.pageIndex", page.to_string()),
                        ("page.pageSize", PAGE_SIZE.to_string()),
                        ("page.orders[0].asc", "true".into()),
                        ("page.orders[0].field", "courBeginTime".into()),
                        ("schoolOpenStatusFlag", "false".into()),
                    ],
                )
                .await?;
            let data = resource_data(&payload)?;
            let records = data
                .get("records")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AppError::Upstream("新视频列表响应缺少 records，不能判断为没有录像".into())
                })?;
            for record in records {
                if let Some(class_id) = value_id(record, "teclId")
                    && class_id != session.teaching_class_id
                {
                    return Err(AppError::Upstream(
                        "新视频平台返回了其他教学班的录像".into(),
                    ));
                }
                let lesson = resource_lesson(record, lessons.len())?;
                if !seen.insert(lesson.video_id.clone()) {
                    return Err(AppError::Upstream(
                        "新视频列表重复分页，未能确认完整列表".into(),
                    ));
                }
                lessons.push(lesson);
            }
            if !has_next_page(data, page, lessons.len(), records.len())? {
                lessons.sort_by(|a, b| a.begin_time.cmp(&b.begin_time));
                return Ok(lessons);
            }
        }
        Err(AppError::Upstream(
            "新视频列表分页超过安全上限，未返回不完整列表".into(),
        ))
    }

    pub(super) async fn resource_video_detail(
        &self,
        jar: Arc<Jar>,
        session: &VideoSession,
        lesson_id: &str,
    ) -> AppResult<VideoDetail> {
        let client = build_client(jar, Policy::none())?;
        let payload = self
            .resource_get(
                &client,
                &session.token,
                "/v1/course_vod_urls_new",
                &[("courseId", lesson_id.into())],
            )
            .await?;
        resource_detail(resource_data(&payload)?, lesson_id)
    }
}

fn validate_adapter_action(url: &Url, adapter: &str, suffix: &str) -> AppResult<()> {
    validate_video_action(url, adapter)?;
    let expected = format!(
        "{}{suffix}",
        Url::parse(adapter)?.path().trim_end_matches('/')
    );
    if url.path() != expected || url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::Upstream(
            "新视频授权表单指向了未识别的入口".into(),
        ));
    }
    Ok(())
}

async fn resource_token_from_response(
    response: reqwest::Response,
    config: &Config,
) -> AppResult<String> {
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(AppError::VideoUnavailable(
            "新视频平台拒绝了授权，请从 Canvas 官网核对课程权限后重试".into(),
        ));
    }
    if !(response.status().is_success() || response.status().is_redirection()) {
        return Err(AppError::Upstream(format!(
            "新视频授权返回 HTTP {}",
            response.status()
        )));
    }
    let target = match response.headers().get(header::LOCATION) {
        Some(location) => response.url().join(
            location
                .to_str()
                .map_err(|_| AppError::Upstream("新视频授权跳转无效".into()))?,
        )?,
        None => response.url().clone(),
    };
    resource_token(&target, &config.resource_video_api)
}

fn resource_token(target: &Url, api: &str) -> AppResult<String> {
    // Observed launch: <resource-api>-ui/#/lms/launch?jwt_token=...
    let ui = format!("{}-ui", api.trim_end_matches('/'));
    validate_video_action(target, &ui)?;
    redirect_parameter(target.as_str(), "jwt_token")
        .filter(|token| {
            !token.is_empty()
                && token.len() <= 16_384
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
        .ok_or_else(|| AppError::Upstream("新视频授权响应缺少有效 jwt_token".into()))
}

fn number(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

fn resource_data(payload: &Value) -> AppResult<&Value> {
    if payload.get("code").and_then(Value::as_str) == Some("LMS_TEACHING_CLASS_NOT_FOUND") {
        return Err(AppError::VideoNotScheduled);
    }
    if number(payload, "status") != Some(200)
        || payload.get("ok").and_then(Value::as_bool) == Some(false)
    {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("响应状态异常");
        return Err(AppError::Upstream(format!(
            "新视频平台：{}",
            message.chars().take(300).collect::<String>()
        )));
    }
    payload
        .get("data")
        .filter(|data| data.is_object())
        .ok_or_else(|| AppError::Upstream("新视频平台响应缺少有效 data".into()))
}

fn teaching_class_id(payload: &Value) -> AppResult<String> {
    let data = resource_data(payload)?;
    if data.get("launchType").and_then(Value::as_str) != Some("CANVAS_RECORD") {
        return Err(AppError::Upstream(
            "新视频平台返回了不支持的入口类型".into(),
        ));
    }
    let id = data
        .get("canvasRecord")
        .and_then(|v| value_id(v, "teachingClassId"))
        .ok_or_else(|| AppError::Upstream("新视频平台授权缺少教学班 ID".into()))?;
    validate_canvas_id(&id)?;
    Ok(id)
}

fn has_next_page(data: &Value, page: usize, total: usize, count: usize) -> AppResult<bool> {
    if let Some(current) = number(data, "pageIndex")
        && current != page as i64
    {
        return Err(AppError::Upstream("新视频平台返回了错误的页码".into()));
    }
    let row_count = number(data, "rowCount").filter(|n| *n >= 0);
    if row_count.is_some_and(|n| total > n as usize) {
        return Err(AppError::Upstream("新视频列表总数与记录数不一致".into()));
    }
    let has_more = row_count
        .map(|n| total < n as usize)
        .or_else(|| number(data, "pageCount").map(|n| page < n as usize))
        .unwrap_or(count >= PAGE_SIZE);
    if has_more && count == 0 {
        return Err(AppError::Upstream(
            "新视频列表分页提前结束，未返回不完整列表".into(),
        ));
    }
    Ok(has_more)
}

fn resource_lesson(value: &Value, index: usize) -> AppResult<Lesson> {
    let id = value_id(value, "id")
        .ok_or_else(|| AppError::Upstream("新视频列表记录缺少课次 ID".into()))?;
    validate_resource_id(&id)?;
    let string = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(key).and_then(Value::as_str))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    // Match the official player's U(record) && record.vodClickEnable gate.
    let status = number(value, "vodDisplayStatus");
    let ready = status
        .map(|n| [3, 5, 6].contains(&n))
        .unwrap_or_else(|| number(value, "clroType") == Some(3));
    let clickable = value.get("vodClickEnable").and_then(Value::as_bool) == Some(true)
        || number(value, "vodClickEnable") == Some(1);
    let title = string(&["courName", "subjName"]);
    Ok(Lesson {
        video_id: id,
        title: if title.is_empty() {
            format!("第 {:02} 讲", index + 1)
        } else {
            title
        },
        begin_time: string(&["courBeginTime"]),
        end_time: string(&["courEndTime"]),
        classroom: string(&["clroName"]),
        audit_status: status.unwrap_or_default(),
        available: ready && clickable,
        source: SOURCE.into(),
    })
}

fn resource_detail(data: &Value, lesson_id: &str) -> AppResult<VideoDetail> {
    if value_id(data, "id").as_deref() != Some(lesson_id) {
        return Err(AppError::Upstream(
            "新视频平台返回的课次与请求不一致".into(),
        ));
    }
    if let Some(message) = data
        .get("errorMsg")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Err(AppError::VideoUnavailable(
            message.chars().take(300).collect(),
        ));
    }
    let views = data
        .get("courseVodViewList")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Upstream("新视频详情缺少 courseVodViewList".into()))?;
    let mut tracks = Vec::new();
    for view in views {
        let Some(view_num) = number(view, "viewNum") else {
            continue;
        };
        let code = match view_num {
            1 => 0,
            3 => 1,
            4 => 2,
            5 => 3,
            7 => 4,
            _ => continue,
        };
        let Some(url) = view
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        tracks.push(VideoTrack {
            id: format!("{lesson_id}-{view_num}"),
            view: code,
            url: url.into(),
        });
    }
    if tracks.is_empty() {
        return Err(AppError::VideoUnavailable(
            "新视频平台没有返回可用的教师、课件或合成分轨".into(),
        ));
    }
    Ok(VideoDetail { tracks })
}

pub(super) fn resolve_resource_track(
    lesson: &Lesson,
    track: &VideoTrack,
    config: &Config,
) -> AppResult<ResolvedVideo> {
    if track.url.trim().is_empty() {
        return Err(AppError::VideoUnavailable("该画面没有下载地址".into()));
    }
    let url = Url::parse(track.url.trim())?;
    validate_generated_url(&url)?;
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !["mp4", "webm", "m4v", "flv", "mov", "mkv"].contains(&extension.as_str()) {
        return Err(AppError::VideoUnavailable("该画面只提供流式播放或未识别的媒体地址，目前无法直接下载；请从 Canvas 官网播放或使用官方“下载视频”功能".into()));
    }
    // Never attach jwt-token to a CDN URL or expose the API authorization in a
    // browser URL. The school-provided signed media URL supports browser-first
    // downloads; the existing session-owned proxy ticket is the fallback.
    let filename = safe_filename(&format!(
        "{}_{}_{}_{}.{}",
        lesson.title,
        lesson.begin_time.get(..10).unwrap_or_default(),
        track_label(track.view),
        lesson.video_id,
        extension
    ));
    Ok(ResolvedVideo {
        upstream_url: url,
        filename,
        headers: vec![(
            "referer".into(),
            format!("{}-ui/", config.resource_video_api.trim_end_matches('/')),
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::video::tests::lti_test_config;
    use serde_json::json;

    #[tokio::test]
    async fn launch_denials_are_permission_errors_and_gateway_failures_are_transient() {
        for status in [
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
        ] {
            let response: reqwest::Response = axum::http::Response::builder()
                .status(status)
                .body(String::new())
                .unwrap()
                .into();
            let error = resource_token_from_response(response, &lti_test_config())
                .await
                .unwrap_err();
            assert_eq!(error.is_transient(), status.is_server_error());
            assert_eq!(
                matches!(error, AppError::VideoUnavailable(_)),
                status.is_client_error()
            );
        }
    }

    #[test]
    fn resource_launch_form_is_validated_against_the_adapter() {
        let config = lti_test_config();
        let page = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"<form method="post" action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record">
          <input type="hidden" name="id_token" value="fixture-token">
          <input type="hidden" name="state" value="fixture-state">
        </form>"#;
        let Some(LtiFormStep::Launch(LtiLaunch { url, fields })) =
            lti_form_step(html, &page, &config).unwrap()
        else {
            panic!("missing launch")
        };
        assert_eq!(fields["state"], "fixture-state");
        assert!(validate_adapter_action(&url, &config.video_lti_adapter, LAUNCH_PATH).is_ok());
        assert!(validate_video_action(&url, &config.resource_video_api).is_err());
        for target in [
            "https://evil.example/jy-lti-adapter/lti/canvas/launch/canvas-record",
            "https://v.sjtu.edu.cn/jy-lti-adapter-evil/lti/canvas/launch/canvas-record",
            "https://v.sjtu.edu.cn/jy-lti-adapter/other",
            "https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record?next=https://evil.example",
        ] {
            assert!(
                validate_adapter_action(
                    &Url::parse(target).unwrap(),
                    &config.video_lti_adapter,
                    LAUNCH_PATH
                )
                .is_err()
            );
        }
    }

    #[test]
    fn jwt_must_come_from_expected_resource_ui() {
        let api = lti_test_config().resource_video_api;
        let good = Url::parse(&format!(
            "{api}-ui/#/lms/launch?jwt_token=header.payload.signature"
        ))
        .unwrap();
        assert_eq!(
            resource_token(&good, &api).unwrap(),
            "header.payload.signature"
        );
        for target in [
            "https://evil.example/jy-application-resourcemanage-ui/#/lms/launch?jwt_token=abc",
            "https://v.sjtu.edu.cn/jy-application-resourcemanage-ui-evil/#/lms/launch?jwt_token=abc",
            "https://v.sjtu.edu.cn/jy-application-resourcemanage-ui/#/lms/launch?jwt_token=",
            "https://v.sjtu.edu.cn/jy-application-resourcemanage-ui/#/lms/launch?jwt_token=bad%0Atoken",
        ] {
            assert!(resource_token(&Url::parse(target).unwrap(), &api).is_err());
        }
    }

    #[test]
    fn context_is_required_and_missing_class_is_not_an_outage() {
        let ok = json!({"status":200,"data":{"launchType":"CANVAS_RECORD","canvasRecord":{"teachingClassId":2883}}});
        assert_eq!(teaching_class_id(&ok).unwrap(), "2883");
        let missing = json!({"status":500,"code":"LMS_TEACHING_CLASS_NOT_FOUND","data":null});
        assert!(matches!(
            resource_data(&missing),
            Err(AppError::VideoNotScheduled)
        ));
        for invalid in [
            json!({"status":500,"data":{"records":[]}}),
            json!({"status":200,"ok":false,"data":{"records":[]}}),
            json!({"status":200,"data":null}),
            json!({"data":{"records":[]}}),
        ] {
            assert!(resource_data(&invalid).is_err());
        }
        assert!(teaching_class_id(&json!({"status":200,"data":{"launchType":"OTHER"}})).is_err());
    }

    #[test]
    fn list_availability_matches_official_player() {
        for status in [0, 1, 2, 3, 4, 5, 6, 7] {
            for clickable in [false, true] {
                let record = json!({"id":123,"courName":"测试课次","courBeginTime":"2026-08-20 09:00:00","clroName":"教室","vodDisplayStatus":status,"vodClickEnable":clickable});
                let lesson = resource_lesson(&record, 0).unwrap();
                assert_eq!(lesson.available, [3, 5, 6].contains(&status) && clickable);
                assert_eq!(lesson.video_id, "123");
                assert_eq!(lesson.title, "测试课次");
            }
        }
        assert!(
            !resource_lesson(&json!({"id":1,"vodDisplayStatus":3}), 0)
                .unwrap()
                .available
        );
        assert!(
            resource_lesson(&json!({"id":1,"clroType":3,"vodClickEnable":1}), 0)
                .unwrap()
                .available
        );
        assert!(resource_lesson(&json!({"courName":"missing id"}), 0).is_err());
    }

    #[test]
    fn pagination_does_not_silently_truncate() {
        assert!(
            !has_next_page(&json!({"pageIndex":1,"rowCount":0,"pageCount":0}), 1, 0, 0).unwrap()
        );
        assert!(has_next_page(&json!({"pageIndex":1,"rowCount":250}), 1, 100, 100).unwrap());
        assert!(!has_next_page(&json!({"pageIndex":3,"rowCount":250}), 3, 250, 50).unwrap());
        assert!(has_next_page(&json!({"pageIndex":1,"rowCount":250}), 1, 0, 0).is_err());
        assert!(has_next_page(&json!({"pageIndex":1}), 2, 100, 0).is_err());
        assert!(has_next_page(&json!({"rowCount":1}), 1, 2, 2).is_err());
    }

    #[test]
    fn tracks_map_new_view_numbers_without_exposing_jwt() {
        let data = json!({"id":123,"courseVodViewList":[
            {"viewNum":1,"url":"https://media.example/teacher.mp4?signature=fixture"},
            {"viewNum":5,"url":"https://media.example/slides.m3u8"},
            {"viewNum":7,"url":"https://media.example/composite.mp4"}
        ]});
        let detail = resource_detail(&data, "123").unwrap();
        let tracks = &detail.tracks;
        assert_eq!(
            tracks.iter().map(|t| t.view).collect::<Vec<_>>(),
            vec![0, 3, 4]
        );
        let lesson = resource_lesson(&json!({"id":123}), 0).unwrap();
        let resolved = resolve_resource_track(&lesson, &tracks[0], &lti_test_config()).unwrap();
        assert_eq!(
            resolved.upstream_url.as_str(),
            "https://media.example/teacher.mp4?signature=fixture"
        );
        assert!(
            !resolved
                .headers
                .iter()
                .any(|(key, _)| key == "jwt-token" || key == "token")
        );
        assert!(resolve_resource_track(&lesson, &tracks[1], &lti_test_config()).is_err());
        assert!(resource_detail(&data, "456").is_err());
        let mut private_track = tracks[0].clone();
        private_track.url = "https://127.0.0.1/secret.mp4".into();
        assert!(resolve_resource_track(&lesson, &private_track, &lti_test_config()).is_err());
    }

    #[tokio::test]
    async fn resource_list_follows_pages_with_server_only_token() {
        use axum::{Json, Router, extract::Query, http::HeaderMap, routing::get};
        let app = Router::new().route("/v1/subject_vod_list_new", get(|headers: HeaderMap, Query(query): Query<HashMap<String,String>>| async move {
            assert_eq!(headers["jwt-token"], "fixture-token");
            assert_eq!(query["teclIds"], "10");
            assert_eq!(query["schoolOpenStatusFlag"], "false");
            assert_eq!(query["page.orders[0].field"], "courBeginTime");
            assert!(!query.contains_key("jwt_token"));
            let page: i64 = query["page.pageIndex"].parse().unwrap();
            Json(json!({"status":200,"data":{"pageIndex":page,"rowCount":2,"records":[{"id":page,"teclId":10,"vodDisplayStatus":3,"vodClickEnable":true}]}}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = lti_test_config();
        config.resource_video_api = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let service = VideoService::new(Arc::new(config));
        let session = VideoSession {
            token: "fixture-token".into(),
            teaching_class_id: "10".into(),
        };
        let result = service
            .resource_lessons(Arc::new(Jar::default()), &session)
            .await;
        task.abort();
        let lessons = result.unwrap();
        assert_eq!(lessons.len(), 2);
        assert!(lessons.iter().all(|lesson| lesson.available));
    }
}
