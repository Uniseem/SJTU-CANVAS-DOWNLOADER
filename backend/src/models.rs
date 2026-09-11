use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasProfile {
    #[serde(deserialize_with = "deserialize_stringish")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub authenticated: bool,
    pub demo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<CanvasProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    #[serde(deserialize_with = "deserialize_stringish")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub course_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teacher: Option<String>,
    #[serde(default)]
    pub enrollment_state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub course_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points_possible: Option<f64>,
    pub submitted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub profile: CanvasProfile,
    pub courses: Vec<Course>,
    pub todos: Vec<TodoItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasFile {
    #[serde(deserialize_with = "deserialize_stringish")]
    pub id: String,
    pub display_name: String,
    pub filename: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points_possible: Option<f64>,
    pub submission_state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lesson {
    pub video_id: String,
    pub title: String,
    pub begin_time: String,
    pub end_time: String,
    pub classroom: String,
    pub audit_status: i64,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoSizeStatus {
    Ready,
    Missing,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTrackSize {
    pub status: VideoSizeStatus,
    pub size: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LessonSizes {
    pub video_id: String,
    pub tracks: std::collections::BTreeMap<String, VideoTrackSize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTrack {
    #[serde(default, deserialize_with = "deserialize_stringish")]
    pub id: String,
    #[serde(default)]
    pub cdvi_view_num: i64,
    #[serde(default, alias = "rtmpUrlHdv")]
    pub rtmp_url_hdv: String,
    #[serde(default, alias = "rtmpUrlHd")]
    pub rtmp_url_hd: String,
    #[serde(default, alias = "rtmpUrl")]
    pub rtmp_url: String,
}

impl VideoTrack {
    pub fn direct_url(&self) -> Option<&str> {
        [
            self.rtmp_url_hdv.trim(),
            self.rtmp_url_hd.trim(),
            self.rtmp_url.trim(),
        ]
        .into_iter()
        .find(|value| !value.is_empty())
    }
}

pub fn deserialize_stringish<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        value => value.to_string(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDescriptor {
    pub id: String,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_url: Option<String>,
    pub proxy_url: String,
    pub expires_at: String,
    pub source: String,
    pub direct_supported: bool,
}
