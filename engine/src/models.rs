//! Canvas and classroom-video data shared with the apps (snake_case JSON).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasProfile {
    #[serde(deserialize_with = "deserialize_stringish")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// active | invited_or_pending | completed
    #[serde(default)]
    pub enrollment_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// Signed download URL; never sent to the apps.
    #[serde(skip_serializing)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Lesson {
    pub video_id: String,
    pub title: String,
    pub begin_time: String,
    pub end_time: String,
    pub classroom: String,
    /// The platform's `vodDisplayStatus`; 3, 5 and 6 mean the recording is open.
    pub audit_status: i64,
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoSizeStatus {
    Ready,
    Missing,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoTrackSize {
    pub status: VideoSizeStatus,
    pub size: Option<u64>,
}

#[derive(Serialize)]
pub struct LessonSizes {
    pub video_id: String,
    pub tracks: std::collections::BTreeMap<String, VideoTrackSize>,
}

/// One camera view of a recording, as the video platform lists it.
#[derive(Clone, Debug, Serialize)]
pub struct VideoTrack {
    pub id: String,
    /// 0 教师 | 1 学生1 | 2 学生2 | 3 PPT | 4 合成 (see `video::track_label`).
    pub view: i64,
    /// The signed media URL; only ever used by the engine.
    #[serde(skip_serializing)]
    pub url: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_profile_reads_canvas_field_names() {
        let profile: CanvasProfile = serde_json::from_value(serde_json::json!({
            "id": 42, "name": "测试用户", "short_name": "测试", "avatar_url": "https://example.test/a.png"
        }))
        .unwrap();
        assert_eq!(profile.id, "42");
        assert_eq!(profile.short_name, "测试");
        assert_eq!(
            profile.avatar_url.as_deref(),
            Some("https://example.test/a.png")
        );
    }
}
