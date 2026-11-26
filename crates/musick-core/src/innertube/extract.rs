use serde_json::Value;

use crate::domain::Thumb;

pub fn get_path<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path {
        cur = match cur {
            Value::Object(map) => map.get(*seg)?,
            Value::Array(arr) => {
                let idx: usize = seg.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

pub fn get_str<'a>(root: &'a Value, path: &[&str]) -> Option<&'a str> {
    get_path(root, path)?.as_str()
}

pub fn get_array<'a>(root: &'a Value, path: &[&str]) -> Option<&'a [Value]> {
    get_path(root, path)?.as_array().map(Vec::as_slice)
}

pub fn runs_text(val: &Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(s) = val.get("simpleText").and_then(Value::as_str) {
        let trimmed = s.trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    let runs = val.get("runs")?.as_array()?;
    let mut out = String::new();
    for r in runs {
        if let Some(t) = r.get("text").and_then(Value::as_str) {
            out.push_str(t);
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn runs_vec(val: &Value) -> Vec<&Value> {
    val.get("runs")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().collect())
        .unwrap_or_default()
}

pub fn extract_thumbs(val: &Value) -> Vec<Thumb> {
    let candidates = get_array(val, &["thumbnails"])
        .or_else(|| get_array(val, &["musicThumbnailRenderer", "thumbnail", "thumbnails"]))
        .or_else(|| {
            get_array(
                val,
                &[
                    "thumbnail",
                    "musicThumbnailRenderer",
                    "thumbnail",
                    "thumbnails",
                ],
            )
        })
        .or_else(|| get_array(val, &["thumbnail", "thumbnails"]))
        .or_else(|| {
            get_array(
                val,
                &["croppedSquareThumbnailRenderer", "thumbnail", "thumbnails"],
            )
        });

    let Some(arr) = candidates else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|item| {
            let url = item.get("url")?.as_str()?.trim();
            if url.is_empty() {
                return None;
            }
            let width = item.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
            let height = item.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
            Some(Thumb {
                url: url.to_string(),
                width,
                height,
            })
        })
        .collect()
}

pub fn parse_duration_str(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.contains(':') {
        return None;
    }
    let parts: Vec<u32> = trimmed
        .split(':')
        .map(|p| p.trim().parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [m, s] => Some(m.saturating_mul(60).saturating_add(*s)),
        [h, m, s] => Some(
            h.saturating_mul(3600)
                .saturating_add(m.saturating_mul(60))
                .saturating_add(*s),
        ),
        _ => None,
    }
}

/// Recursively collect all values stored under `target_key` in DFS order.
pub fn find_all_by_key<'a>(root: &'a Value, target_key: &str, out: &mut Vec<&'a Value>) {
    match root {
        Value::Object(map) => {
            if let Some(found) = map.get(target_key) {
                out.push(found);
            }
            for (k, v) in map {
                if k != target_key {
                    find_all_by_key(v, target_key, out);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                find_all_by_key(item, target_key, out);
            }
        }
        _ => {}
    }
}

pub fn extract_navigation_browse_id(val: &Value) -> Option<String> {
    get_str(val, &["navigationEndpoint", "browseEndpoint", "browseId"])
        .or_else(|| get_str(val, &["browseEndpoint", "browseId"]))
        .map(str::to_string)
}

pub fn extract_navigation_page_type(val: &Value) -> Option<String> {
    get_str(
        val,
        &[
            "navigationEndpoint",
            "browseEndpoint",
            "browseEndpointContextSupportedConfigs",
            "browseEndpointContextMusicConfig",
            "pageType",
        ],
    )
    .or_else(|| {
        get_str(
            val,
            &[
                "browseEndpoint",
                "browseEndpointContextSupportedConfigs",
                "browseEndpointContextMusicConfig",
                "pageType",
            ],
        )
    })
    .map(str::to_string)
}

pub fn extract_watch_video_id(val: &Value) -> Option<String> {
    get_str(val, &["navigationEndpoint", "watchEndpoint", "videoId"])
        .or_else(|| get_str(val, &["watchEndpoint", "videoId"]))
        .or_else(|| get_str(val, &["playlistItemData", "videoId"]))
        .map(str::to_string)
}

pub fn extract_watch_video_type(val: &Value) -> Option<String> {
    get_str(
        val,
        &[
            "navigationEndpoint",
            "watchEndpoint",
            "watchEndpointMusicSupportedConfigs",
            "watchEndpointMusicConfig",
            "musicVideoType",
        ],
    )
    .or_else(|| {
        get_str(
            val,
            &[
                "watchEndpoint",
                "watchEndpointMusicSupportedConfigs",
                "watchEndpointMusicConfig",
                "musicVideoType",
            ],
        )
    })
    .map(str::to_string)
}
