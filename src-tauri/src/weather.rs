use serde_json::{json, Value};

fn weather_code_zh(code: i64) -> &'static str {
    match code {
        0 => "晴",
        1 => "大部晴朗",
        2 => "多云",
        3 => "阴",
        45 | 48 => "有雾",
        51 | 53 | 55 => "毛毛雨",
        56 | 57 => "冻毛毛雨",
        61 => "小雨",
        63 => "中雨",
        65 => "大雨",
        66 | 67 => "冻雨",
        71 => "小雪",
        73 => "中雪",
        75 => "大雪",
        77 => "雪粒",
        80 | 81 | 82 => "阵雨",
        85 | 86 => "阵雪",
        95 => "雷阵雨",
        96 | 99 => "强雷阵雨伴冰雹",
        _ => "阴晴不定",
    }
}

fn day_offset_from_hint(hint: &str) -> usize {
    if hint.contains("大后天") {
        3
    } else if hint.contains("后天") {
        2
    } else if hint.contains("明") {
        1
    } else {
        0
    }
}

pub async fn fetch_free_weather(client: &reqwest::Client, city: &str, date_hint: &str) -> Value {
    let clean_city = city
        .trim()
        .trim_end_matches('市')
        .trim_end_matches('区')
        .trim();
    if clean_city.is_empty() {
        return json!({ "ok": false, "error": "缺少城市名，请先向对方确认想查哪座城市" });
    }

    let geo_url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=zh&format=json",
        url_encode(clean_city)
    );
    let geo_res = match client.get(&geo_url).send().await {
        Ok(r) => match r.json::<Value>().await {
            Ok(v) => v,
            Err(e) => return json!({ "ok": false, "error": format!("天气地理接口解析失败: {e}") }),
        },
        Err(e) => return json!({ "ok": false, "error": format!("暂时连不上免费天气网（可能断网）: {e}") }),
    };

    let loc = match geo_res
        .get("results")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
    {
        Some(l) => l,
        None => return json!({ "ok": false, "error": format!("没查到「{clean_city}」的坐标") }),
    };

    let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let resolved_name = loc
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(clean_city);

    let forecast_url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&current=temperature_2m,weather_code&timezone=auto&forecast_days=5"
    );

    let fc_res = match client.get(&forecast_url).send().await {
        Ok(r) => match r.json::<Value>().await {
            Ok(v) => v,
            Err(e) => return json!({ "ok": false, "error": format!("天气预报解析失败: {e}") }),
        },
        Err(e) => return json!({ "ok": false, "error": format!("天气接口请求失败: {e}") }),
    };

    let idx = day_offset_from_hint(date_hint);
    let daily = &fc_res["daily"];
    let date_str = daily["time"]
        .get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let t_min = daily["temperature_2m_min"]
        .get(idx)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let t_max = daily["temperature_2m_max"]
        .get(idx)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let rain_prob = daily["precipitation_probability_max"]
        .get(idx)
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let code = daily["weather_code"]
        .get(idx)
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let current_temp = fc_res["current"]["temperature_2m"].as_f64();
    let current_code = fc_res["current"]["weather_code"].as_i64();

    json!({
        "ok": true,
        "city": resolved_name,
        "dateHint": if date_hint.trim().is_empty() { "今天" } else { date_hint },
        "date": date_str,
        "condition": weather_code_zh(code),
        "tempMin": (t_min * 10.0).round() / 10.0,
        "tempMax": (t_max * 10.0).round() / 10.0,
        "rainProbabilityPercent": rain_prob,
        "currentTemp": current_temp.map(|t| (t * 10.0).round() / 10.0),
        "currentCondition": current_code.map(weather_code_zh),
        "source": "Open-Meteo (free)"
    })
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.' | b'~') {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}
