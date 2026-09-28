use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    sync::Arc,
    time::Duration,
};

use futures_util::future::try_join_all;
use reqwest::{
    Client, Response, StatusCode,
    cookie::Jar,
    header::{self, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use url::Url;

use crate::{
    config::Config,
    error::{AppError, AppResult},
    http,
    models::{CanvasFile, CanvasProfile, Course},
};

const CANVAS_ACCEPT: &str = "application/json+canvas-string-ids";

pub struct CanvasClient {
    config: Arc<Config>,
    client: Client,
    no_redirect: Client,
}

impl CanvasClient {
    pub fn new(config: Arc<Config>, jar: Arc<Jar>) -> AppResult<Self> {
        Ok(Self {
            client: build_client(jar.clone(), Policy::limited(10))?,
            no_redirect: build_client(jar.clone(), Policy::none())?,
            config,
        })
    }

    pub async fn profile(&self) -> AppResult<CanvasProfile> {
        self.get_json("/api/v1/users/self/profile", &[]).await
    }

    pub async fn courses(&self) -> AppResult<Vec<Course>> {
        let states = ["active", "invited_or_pending", "completed"];
        let batches = try_join_all(states.into_iter().map(|state| async move {
            let values = self
                .paginated_values(
                    "/api/v1/courses",
                    &[
                        ("enrollment_state", state),
                        ("state[]", "unpublished"),
                        ("state[]", "available"),
                        ("state[]", "completed"),
                        ("include[]", "term"),
                        ("include[]", "teachers"),
                        ("include[]", "concluded"),
                    ],
                )
                .await?;
            let courses = values
                .iter()
                .filter_map(|value| course_from_value(value, state))
                .collect::<Vec<_>>();
            Ok::<_, AppError>(courses)
        }))
        .await?;

        Ok(merge_courses(batches.into_iter().flatten()))
    }

    pub async fn files(&self, course_id: &str) -> AppResult<Vec<CanvasFile>> {
        validate_canvas_id(course_id)?;
        let values = self
            .paginated_values(
                &format!("/api/v1/courses/{course_id}/files"),
                &[("sort", "name"), ("order", "asc"), ("include[]", "user")],
            )
            .await?;
        Ok(values.iter().filter_map(file_from_value).collect())
    }

    pub async fn file(&self, course_id: &str, file_id: &str) -> AppResult<CanvasFile> {
        validate_canvas_id(course_id)?;
        validate_canvas_id(file_id)?;
        let value: Value = self
            .get_json(
                &format!("/api/v1/courses/{course_id}/files/{file_id}"),
                &[("include[]", "user"), ("use_verifiers", "true")],
            )
            .await?;
        file_from_value(&value).ok_or_else(|| AppError::Upstream("文件数据格式异常".into()))
    }

    pub async fn resolve_file_url(&self, file: &CanvasFile) -> AppResult<ResolvedFile> {
        let fallback = format!(
            "{}/files/{}/download?download_frd=1",
            self.config.canvas_origin, file.id
        );
        let initial = file.url.as_deref().unwrap_or(&fallback);
        let mut current = absolutize(&self.config.canvas_origin, initial)?;
        validate_generated_url(&current)?;
        let canvas_origin = Url::parse(&self.config.canvas_origin)?;

        // Follow Canvas's own redirects to the signed storage URL.
        for _ in 0..6 {
            if !same_origin(&current, &canvas_origin) {
                break;
            }
            let response = self
                .no_redirect
                .get(current.clone())
                .header(header::ACCEPT, "application/octet-stream,*/*")
                .send()
                .await?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| AppError::Upstream("文件下载跳转缺少 Location".into()))?;
                current = current.join(location)?;
                validate_generated_url(&current)?;
                continue;
            }
            if response.status().is_success() || response.status() == StatusCode::PARTIAL_CONTENT {
                break;
            }
            return Err(map_canvas_status(response.status(), "解析文件下载地址"));
        }

        Ok(ResolvedFile {
            upstream_url: current,
            filename: safe_filename(if file.display_name.is_empty() {
                &file.filename
            } else {
                &file.display_name
            }),
            size: (file.size > 0).then_some(file.size),
        })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> AppResult<T> {
        let response = self
            .client
            .get(format!("{}{}", self.config.canvas_origin, path))
            .query(query)
            .header(header::ACCEPT, CANVAS_ACCEPT)
            .send()
            .await?;
        parse_canvas_json(response, "读取 Canvas 数据").await
    }

    async fn paginated_values(&self, path: &str, query: &[(&str, &str)]) -> AppResult<Vec<Value>> {
        let canvas_origin = Url::parse(&self.config.canvas_origin)?;
        let mut next = Url::parse(&format!("{}{}", self.config.canvas_origin, path))?;
        {
            let mut pairs = next.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
            pairs.append_pair("per_page", "100");
        }
        let mut output = Vec::new();
        let mut visited = HashSet::new();
        for page_index in 0..100 {
            if !same_origin(&next, &canvas_origin) {
                return Err(AppError::Upstream("Canvas 分页地址越界".into()));
            }
            if !visited.insert(next.to_string()) {
                return Err(AppError::Upstream("Canvas 分页地址出现循环".into()));
            }
            let response = self
                .client
                .get(next.clone())
                .header(header::ACCEPT, CANVAS_ACCEPT)
                .send()
                .await?;
            let headers = response.headers().clone();
            let mut page: Vec<Value> = parse_canvas_json(response, "读取 Canvas 列表").await?;
            output.append(&mut page);
            let Some(url) = next_link(&headers) else {
                return Ok(output);
            };
            if page_index == 99 {
                return Err(AppError::Upstream(
                    "Canvas 分页超过安全上限，未返回完整列表".into(),
                ));
            }
            next = Url::parse(&url)
                .or_else(|_| canvas_origin.join(&url))
                .map_err(AppError::from)?;
        }
        Err(AppError::Upstream("Canvas 分页状态异常".into()))
    }
}

pub struct ResolvedFile {
    pub upstream_url: Url,
    pub filename: String,
    pub size: Option<u64>,
}

pub fn build_client(jar: Arc<Jar>, redirect: Policy) -> AppResult<Client> {
    http::apply(Client::builder())
        // The SJTU login frontends currently reset some negotiated HTTP/2
        // connections on Windows; their browser-facing endpoints are stable on
        // HTTP/1.1, which also matches the LTI form and QR websocket handshakes.
        .http1_only()
        .cookie_provider(jar)
        .user_agent(http::user_agent())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(redirect)
        .build()
        .map_err(AppError::internal)
}

async fn parse_canvas_json<T: serde::de::DeserializeOwned>(
    response: Response,
    context: &str,
) -> AppResult<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(map_canvas_status(status, context));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.contains("json") {
        return Err(non_json_canvas_error(response.url(), context));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| AppError::Upstream(format!("{context}响应格式异常：{error}")))
}

fn non_json_canvas_error(url: &Url, context: &str) -> AppError {
    // A maintenance page or malformed response is not proof of logout.
    let path = url.path();
    if path == "/login" || path.starts_with("/login/") || path.starts_with("/jaccount/") {
        AppError::Unauthorized
    } else {
        AppError::Upstream(format!("{context}未返回 JSON 数据，学校服务可能暂时异常"))
    }
}

fn map_canvas_status(status: StatusCode, context: &str) -> AppError {
    match status {
        StatusCode::UNAUTHORIZED => AppError::Unauthorized,
        StatusCode::FORBIDDEN => {
            AppError::Forbidden(format!("{context}：当前账号无权访问，课程可能尚未开放"))
        }
        StatusCode::NOT_FOUND => AppError::NotFound(format!("{context}：资源不存在或无权访问")),
        StatusCode::TOO_MANY_REQUESTS => {
            AppError::Upstream("Canvas 请求过于频繁，请稍后重试".into())
        }
        _ => AppError::Upstream(format!("{context}失败（HTTP {status}）")),
    }
}

fn next_link(headers: &HeaderMap<HeaderValue>) -> Option<String> {
    for value in headers.get_all(header::LINK) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for part in value.split(',') {
            let Some((url, attrs)) = part.trim().split_once(';') else {
                continue;
            };
            if attrs.split(';').any(|attr| attr.trim() == "rel=\"next\"") {
                return Some(
                    url.trim()
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .to_string(),
                );
            }
        }
    }
    None
}

pub fn value_id(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn course_from_value(value: &Value, enrollment_state: &str) -> Option<Course> {
    let id = value_id(value, "id")?;
    let name = optional_string(value, "name")
        .or_else(|| optional_string(value, "course_code"))
        .unwrap_or_else(|| format!("课程 {id}"));
    Some(Course {
        id,
        name,
        course_code: optional_string(value, "course_code").unwrap_or_default(),
        start_at: optional_string(value, "start_at"),
        end_at: optional_string(value, "end_at"),
        term: value
            .get("term")
            .and_then(|term| optional_string(term, "name")),
        teacher: course_teachers(value),
        enrollment_state: enrollment_state.to_string(),
    })
}

fn course_teachers(value: &Value) -> Option<String> {
    let names = value
        .get("teachers")?
        .as_array()?
        .iter()
        .filter_map(|teacher| {
            optional_string(teacher, "display_name")
                .or_else(|| optional_string(teacher, "name"))
                .or_else(|| optional_string(teacher, "short_name"))
        })
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("、"))
}

fn merge_courses(courses: impl IntoIterator<Item = Course>) -> Vec<Course> {
    let mut merged = HashMap::<String, Course>::new();
    for course in courses {
        match merged.entry(course.id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(course);
            }
            Entry::Occupied(mut entry)
                if enrollment_rank(&course.enrollment_state)
                    < enrollment_rank(&entry.get().enrollment_state) =>
            {
                entry.insert(course);
            }
            Entry::Occupied(_) => {}
        }
    }
    let mut courses = merged.into_values().collect::<Vec<_>>();
    courses.sort_by(|left, right| {
        enrollment_rank(&left.enrollment_state)
            .cmp(&enrollment_rank(&right.enrollment_state))
            .then_with(|| right.end_at.cmp(&left.end_at))
            .then_with(|| left.name.cmp(&right.name))
    });
    courses
}

fn file_from_value(value: &Value) -> Option<CanvasFile> {
    let filename = optional_string(value, "filename").unwrap_or_default();
    Some(CanvasFile {
        id: value_id(value, "id")?,
        display_name: optional_string(value, "display_name").unwrap_or_else(|| filename.clone()),
        filename,
        size: value
            .get("size")
            .and_then(|entry| entry.as_u64().or_else(|| entry.as_str()?.parse().ok()))
            .unwrap_or(0),
        content_type: optional_string(value, "content-type")
            .or_else(|| optional_string(value, "content_type")),
        updated_at: optional_string(value, "updated_at"),
        url: optional_string(value, "url"),
    })
}

fn enrollment_rank(value: &str) -> u8 {
    match value {
        "active" => 0,
        "invited_or_pending" => 1,
        "completed" => 2,
        _ => 3,
    }
}

pub fn validate_canvas_id(value: &str) -> AppResult<()> {
    if value.is_empty() || value.len() > 32 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::BadRequest("Canvas 资源 ID 无效".into()));
    }
    Ok(())
}

pub fn safe_filename(value: &str) -> String {
    let mut result = value
        .chars()
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
    result = result
        .trim()
        .trim_end_matches(['.', ' '])
        .chars()
        .take(160)
        .collect();
    if result.is_empty() {
        result = "Canvas-下载".into();
    }
    let stem = result
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered_device = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem
            .as_bytes()
            .last()
            .is_some_and(|digit| matches!(*digit, b'1'..=b'9'));
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered_device {
        result.insert(0, '_');
    }
    result
}

pub fn validate_generated_url(url: &Url) -> AppResult<()> {
    if http::is_test_origin(url) {
        return Ok(());
    }
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Upstream("上游下载地址不安全".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Upstream("上游下载地址缺少主机名".into()))?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err(AppError::Upstream("上游下载地址不安全".into()));
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        let unsafe_ip = match address {
            std::net::IpAddr::V4(ip) => {
                ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
            }
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local()
            }
        };
        if unsafe_ip {
            return Err(AppError::Upstream("上游下载地址指向私有网络".into()));
        }
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn absolutize(origin: &str, value: &str) -> AppResult<Url> {
    Url::parse(value)
        .or_else(|_| Url::parse(origin)?.join(value))
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_is_only_expired_login_when_redirected_to_authentication() {
        for path in ["/login", "/login/openid_connect", "/jaccount/jalogin"] {
            assert!(matches!(
                non_json_canvas_error(
                    &Url::parse(&format!("https://example.test{path}")).unwrap(),
                    "读取数据"
                ),
                AppError::Unauthorized
            ));
        }
        for path in [
            "/api/v1/users/self/profile",
            "/maintenance",
            "/courses/94198",
        ] {
            assert!(matches!(
                non_json_canvas_error(
                    &Url::parse(&format!("https://example.test{path}")).unwrap(),
                    "读取数据"
                ),
                AppError::Upstream(_)
            ));
        }
    }

    #[test]
    fn course_permission_denial_is_not_session_expiration() {
        assert!(matches!(
            map_canvas_status(StatusCode::FORBIDDEN, "课程文件"),
            AppError::Forbidden(_)
        ));
        assert!(matches!(
            map_canvas_status(StatusCode::UNAUTHORIZED, "课程文件"),
            AppError::Unauthorized
        ));
    }

    #[test]
    fn parses_canvas_next_link() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "<https://oc.sjtu.edu.cn/api/v1/courses?page=1>; rel=\"current\", <https://oc.sjtu.edu.cn/api/v1/courses?page=2>; rel=\"next\"",
            ),
        );
        assert_eq!(
            next_link(&headers).as_deref(),
            Some("https://oc.sjtu.edu.cn/api/v1/courses?page=2")
        );
    }

    #[test]
    fn parses_next_link_across_multiple_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::LINK,
            HeaderValue::from_static(
                "<https://oc.sjtu.edu.cn/api/v1/courses?page=1>; rel=\"current\"",
            ),
        );
        headers.append(
            header::LINK,
            HeaderValue::from_static(
                "<https://oc.sjtu.edu.cn/api/v1/courses?page=2>; rel=\"next\"",
            ),
        );
        assert_eq!(
            next_link(&headers).as_deref(),
            Some("https://oc.sjtu.edu.cn/api/v1/courses?page=2")
        );
    }

    #[test]
    fn parses_course_teachers() {
        let value = serde_json::json!({
            "id": "42",
            "name": "测试课程",
            "teachers": [
                { "display_name": "张老师" },
                { "name": "李老师" }
            ]
        });
        let course = course_from_value(&value, "active").expect("course should parse");
        assert_eq!(course.teacher.as_deref(), Some("张老师、李老师"));
    }

    #[test]
    fn merge_prefers_the_most_accessible_enrollment_state() {
        let make_course = |state: &str| Course {
            id: "42".into(),
            name: "测试课程".into(),
            course_code: "TEST42".into(),
            start_at: None,
            end_at: None,
            term: None,
            teacher: None,
            enrollment_state: state.into(),
        };
        let merged = merge_courses([
            make_course("completed"),
            make_course("invited_or_pending"),
            make_course("active"),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].enrollment_state, "active");
    }

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(safe_filename("../../CON?.mp4"), ".._.._CON_.mp4");
        assert_eq!(safe_filename("COM9.txt"), "_COM9.txt");
        assert_eq!(safe_filename("LPT8"), "_LPT8");
        assert_eq!(safe_filename("  "), "Canvas-下载");
    }

    #[test]
    fn rejects_private_download_urls() {
        let url = Url::parse("https://127.0.0.1/secret").unwrap();
        assert!(validate_generated_url(&url).is_err());
    }
}
