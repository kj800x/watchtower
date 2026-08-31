use chrono::{DateTime, Utc};

pub fn format_relative_time(time: Option<DateTime<Utc>>) -> String {
    let Some(time) = time else {
        return "never".to_string();
    };

    let delta = Utc::now().signed_duration_since(time);
    if delta.num_seconds() < 0 {
        return "in the future".to_string();
    }
    if delta.num_seconds() < 60 {
        return "just now".to_string();
    }
    if delta.num_minutes() < 60 {
        return format!("{}m ago", delta.num_minutes());
    }
    if delta.num_hours() < 48 {
        return format!("{}h ago", delta.num_hours());
    }
    format!("{}d ago", delta.num_days())
}

/// Shorten "sha256:71388a4556..." to "71388a455697" for display.
pub fn format_short_digest(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    hex.chars().take(12).collect()
}
