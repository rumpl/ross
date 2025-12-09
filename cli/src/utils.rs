pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

pub fn format_timestamp(ts: &prost_types::Timestamp) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let secs = ts.seconds as u64;
    let nanos = ts.nanos as u32;

    if let Some(time) = UNIX_EPOCH.checked_add(Duration::new(secs, nanos)) {
        let datetime: chrono::DateTime<chrono::Utc> = time.into();
        datetime.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
    } else {
        "invalid timestamp".to_string()
    }
}
