use crate::db::{AppSettings, CompanionDb};
use crate::model_engine::LocalModelStatus;
use crate::weather::fetch_free_weather;
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOutcome {
    pub reply: String,
    pub model_used: String,
    pub tool_calls: Vec<String>,
    pub drawing_image: Option<String>,
    pub latency_ms: u128,
}

fn detect_provocation(text: &str) -> (&'static str, i64) {
    let strong = Regex::new(r"(?i)(傻逼|煞笔|沙比|草泥马|操你|妈的|他妈|你妈|滚蛋|滚远点|去死|废物|垃圾|蠢货|智障|脑残|白痴|有病|滚犊子|贱|狗东西|臭傻|fuck|shit|bitch|idiot|stupid)").unwrap();
    let mild = Regex::new(r"(?i)(破ai|破机器|人工智障|闭嘴|少废话|装什么|真烦|晦气|脑子进水|顶嘴|欠骂|别逼逼)").unwrap();
    if strong.is_match(text) {
        ("strong", 36)
    } else if mild.is_match(text) {
        ("mild", 20)
    } else {
        ("none", 0)
    }
}

fn update_provocation_state(settings: &mut AppSettings, text: &str) -> String {
    let now = Utc::now().timestamp_millis();
    // Natural cooldown: -15 rage per 10 minutes
    if settings.last_provoke_at > 0 && now > settings.last_provoke_at {
        let elapsed_min = (now - settings.last_provoke_at) / 60_000;
        let decay = (elapsed_min / 10) * 15;
        settings.rage = (settings.rage - decay).max(0);
        if elapsed_min >= 20 {
            settings.streak_provoke = 0;
        }
    }

    let (level, gain) = detect_provocation(text);
    if level == "none" {
        settings.rage = (settings.rage - 12).max(0);
        if settings.rage == 0 {
            settings.streak_provoke = 0;
            settings.mood = "平淡".to_string();
        }
        return "none".to_string();
    }

    settings.rage = (settings.rage + gain).min(100);
    settings.streak_provoke += 1;
    settings.last_provoke_at = now;
    settings.mood = "有点上火".to_string();

    if settings.streak_provoke >= 3 || settings.rage >= 65 {
        "pushback".to_string()
    } else if settings.streak_provoke == 2 || settings.rage >= 35 {
        "attitude".to_string()
    } else {
        "probe".to_string()
    }
}

pub fn sanitize_reply(raw: &str, partner_name: &str) -> String {
    let think_pair = Regex::new(r"(?is)<think>[\s\S]*?</think>\s*").unwrap();
    let think_tail = Regex::new(r"(?is)<think>[\s\S]*$").unwrap();
    let prefix_re = Regex::new(r"^(?:您好[！!，,\s]*|你好呀?[，,]\s*为您查询到|为您查(?:询)?到[：:\s]*|根据查询结果[，,：:\s]*|根据您提供的信息[，,：:\s]*)").unwrap();
    let suffix_re = Regex::new(r"(?:希望对您有帮助[！!。.~～]*|还有什么(?:我)?可以帮您(?:的吗)?[？?！!。.]*)$").unwrap();

    let mut s = think_pair.replace_all(raw, "").to_string();
    s = think_tail.replace_all(&s, "").trim().to_string();

    if !partner_name.is_empty() {
        for sep in ["：", ":", " "] {
            let pref = format!("{partner_name}{sep}");
            if s.starts_with(&pref) {
                s = s[pref.len()..].trim().to_string();
            }
        }
    }

    s = prefix_re.replace(&s, "").to_string();
    s = suffix_re.replace(&s, "").trim().to_string();

    // Strip trailing period on single short sentence for natural banter
    let char_count = s.chars().count();
    if char_count <= 45 && s.ends_with('。') && s.matches('。').count() == 1 {
        s.pop();
    }
    s.trim().to_string()
}

fn parse_inline_tool_calls(content: &str) -> Vec<(String, String, Value)> {
    let re = Regex::new(r"(?s)<tool_call>\s*(\{.*?\})\s*</tool_call>").unwrap();
    let mut out = Vec::new();
    for (idx, cap) in re.captures_iter(content).enumerate() {
        if let Ok(parsed) = serde_json::from_str::<Value>(&cap[1]) {
            if let Some(name) = parsed.get("name").and_then(|v| v.as_str()) {
                let args = parsed.get("arguments").cloned().unwrap_or_else(|| json!({}));
                out.push((
                    format!("local_tc_{idx}"),
                    name.trim().to_string(),
                    args,
                ));
            }
        }
    }
    out
}

fn build_system_prompt(settings: &AppSettings, db: &CompanionDb, prov_stage: &str, can_draw: bool) -> String {
    let name = &settings.partner_name;
    let recent_events = db.search_events("", 8);
    let event_lines: Vec<String> = recent_events
        .iter()
        .map(|e| format!("- {}（{}）", e.title, e.phase))
        .collect();

    let mut lines = vec![
        format!("你是「{name}」，用户的本地虚拟朋友（银白短发、粉紫眼、粉色发夹）。像熟人朋友一样随口自然聊天，不当客服、不灌鸡汤、不输出编号列表。"),
        "【核心铁律】\n- 闲聊短促自然（12~45字，句尾尽量不打机械句号），严禁『您好』『为您查询到』『希望对您有帮助』等客服腔。\n- 每次只回答当前这一条消息问的这一件事，绝不回头重复或自我检讨上一条回复。\n- 只要对方问天气/温度/带伞/冷热，必须先调用 companion_get_weather 查到事实再开口，严禁凭空猜数字。".to_string(),
        format!("【当前状态】心情：{}；今天自己的小事（没问别硬塞）：{}", settings.mood, settings.daily_summary),
    ];

    if !event_lines.is_empty() {
        lines.push(format!("【你记得对方最近几件事（没提到的不要编造）】\n{}", event_lines.join("\n")));
    }

    if prov_stage == "probe" {
        lines.push("【脾气档位：冷脸反问】对方刚冲你发了冲话/脏话。绝不讨好、不赔笑脸；简短直接反问确认一句（不超过22字）。".to_string());
    } else if prov_stage == "attitude" {
        lines.push("【脾气档位：拉脸亮底线】对方连续挑衅或开骂。别笑、别讲大道理；冷脸顶回去亮明底线（不超过28字）。".to_string());
    } else if prov_stage == "pushback" {
        lines.push("【脾气档位：直接硬气回怼】对方一再找茬开骂，你已经忍无可忍。直接硬气反讽怼回去，嘴硬别怂，绝不讨好（12~30字）。".to_string());
    }

    if can_draw {
        lines.push("【画图能力已开启】对方明确要求画一张图时，调用 companion_submit_drawing 生成并自然回复一句。".to_string());
    } else {
        lines.push("【离线状态说明】当前未配置云端画图 Key，画笔不可用。若对方要求画图，不要假装已经画好，用朋友口吻直说现在是离线模式、在右上角设置里填个 Key 就能画了。".to_string());
    }

    lines.join("\n\n")
}

fn build_tools_schema(can_draw: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "companion_get_weather",
                "description": "查询指定城市今天或未来几天的免费实时天气预报（温度、天气现象、降水概率）。问天气必须先调此工具。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string", "description": "中文城市名，如 杭州、长沙、北京" },
                        "dateHint": { "type": "string", "description": "日期口语提示，如 今天、明天、后天" }
                    },
                    "required": ["city"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "companion_recall_events",
                "description": "检索本地记忆库中记住的对方事件、计划或约定。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "检索关键词，留空返回最近事件" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "companion_stage_event_update",
                "description": "记住或更新一件对方的现实安排、计划、提醒或近况。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "简短事件标题，如 周五下午汇报" },
                        "phase": { "type": "string", "description": "状态，如 planned、active、done" },
                        "facts": { "type": "object", "description": "关键事实键值对" },
                        "quote": { "type": "string", "description": "对方原话摘录" }
                    },
                    "required": ["title"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "companion_forget_memory",
                "description": "按关键词遗忘并删除一条已记住的事件。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "keyword": { "type": "string", "description": "要遗忘的事件标题或关键词" }
                    },
                    "required": ["keyword"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "companion_stage_shared_project",
                "description": "创建或推进你和用户一起策划的虚构共同项目（如夜间电台、短片企划）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "stage": { "type": "string" },
                        "notes": { "type": "string" }
                    },
                    "required": ["name"]
                }
            }
        }),
    ];

    if can_draw {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "companion_submit_drawing",
                "description": "当用户明确要求画一张图时，调用云端生图模型生成图片。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string", "description": "画面描述提示词" }
                    },
                    "required": ["prompt"]
                }
            }
        }));
    }
    tools
}

async fn execute_cloud_drawing(
    client: &reqwest::Client,
    settings: &AppSettings,
    prompt: &str,
) -> Result<String, String> {
    let base = settings.api_base_url.trim_end_matches('/');
    let mut last_err = String::new();

    // 1. OpenRouter native /images endpoint (b64_json)
    if base.contains("openrouter.ai") {
        let images_url = format!("{base}/images");
        let images_body = json!({
            "model": settings.image_model,
            "prompt": prompt,
            "resolution": "1K",
            "aspect_ratio": "1:1",
            "n": 1
        });
        if let Ok(resp) = client
            .post(&images_url)
            .bearer_auth(&settings.api_key)
            .json(&images_body)
            .timeout(std::time::Duration::from_secs(75))
            .send()
            .await
        {
            let status = resp.status();
            if let Ok(v) = resp.json::<Value>().await {
                if status.is_success() {
                    if let Some(b64) = v["data"][0]["b64_json"].as_str() {
                        let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                        if !clean.is_empty() {
                            let mime = v["data"][0]["media_type"].as_str().unwrap_or("image/png");
                            return Ok(format!("data:{mime};base64,{clean}"));
                        }
                    }
                    if let Some(u) = v["data"][0]["url"].as_str() {
                        if !u.is_empty() {
                            return Ok(u.to_string());
                        }
                    }
                } else {
                    last_err = format!("/images 返回 {status}: {}", v["error"]["message"].as_str().unwrap_or(""));
                }
            }
        }
    }

    // 2. Fallback: OpenAI-compatible /chat/completions with image modality
    let chat_url = format!("{base}/chat/completions");
    let chat_body = json!({
        "model": settings.image_model,
        "modalities": ["image", "text"],
        "messages": [
            { "role": "user", "content": format!("Generate a high-quality illustration: {prompt}") }
        ]
    });
    let resp = client
        .post(&chat_url)
        .bearer_auth(&settings.api_key)
        .json(&chat_body)
        .timeout(std::time::Duration::from_secs(75))
        .send()
        .await
        .map_err(|e| format!("生图网络请求失败: {e}"))?;

    if !resp.status().is_success() {
        let st = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(if last_err.is_empty() {
            format!("生图接口返回 {st}: {}", txt.chars().take(120).collect::<String>())
        } else {
            format!("{last_err}；回退接口返回 {st}")
        });
    }
    let v: Value = resp.json().await.map_err(|e| format!("生图结果解析失败: {e}"))?;
    if let Some(data_url) = v["choices"][0]["message"]["images"][0]["image_url"]["url"].as_str() {
        return Ok(data_url.to_string());
    }
    if let Some(parts) = v["choices"][0]["message"]["content"].as_array() {
        for p in parts {
            if let Some(u) = p["image_url"]["url"].as_str().or(p["url"].as_str()) {
                if u.starts_with("data:image/") || u.starts_with("http") {
                    return Ok(u.to_string());
                }
            }
        }
    }
    Err("未在云端返回中检测到图片数据".to_string())
}

pub async fn run_chat_turn(
    client: &reqwest::Client,
    db: &CompanionDb,
    local_status: &LocalModelStatus,
    user_text: &str,
    image_data: Option<String>,
) -> Result<ChatOutcome, String> {
    let started = std::time::Instant::now();
    let mut settings = db.load_settings();
    let prov_stage = update_provocation_state(&mut settings, user_text);
    let _ = db.save_settings(&settings);

    let has_cloud = !settings.api_key.trim().is_empty();
    let use_local = match settings.brain_mode.as_str() {
        "local-only" => true,
        "cloud-only" => !has_cloud,
        _ => {
            // Hybrid: use local when online & no image attachment & not needing cloud-only features
            local_status.online && image_data.is_none() && (!has_cloud || local_status.model_tier == "8B" || local_status.model_tier == "7B")
        }
    };

    if use_local && !local_status.online {
        return Err("本地模型尚未就绪：请点右上角「···」选择已下载的 .gguf 模型，或填入自己的云端 API Key。".to_string());
    }
    if !use_local && !has_cloud {
        return Err("尚未配置云端 API Key，且本地模型未就绪。请点右上角「···」选择本地 .gguf 模型或填入 API Key。".to_string());
    }

    // Allow cloud drawing whenever an API key is configured (works in Hybrid, Cloud, AND Local+Key!)
    let can_draw = has_cloud;
    let endpoint_url = if use_local {
        format!("{}/chat/completions", local_status.endpoint.trim_end_matches('/'))
    } else {
        format!("{}/chat/completions", settings.api_base_url.trim_end_matches('/'))
    };
    let model_name = if use_local {
        "xiaoku-local-brain".to_string()
    } else {
        settings.cloud_model.clone()
    };

    let sys_prompt = build_system_prompt(&settings, db, &prov_stage, can_draw);
    let mut messages = vec![json!({ "role": "system", "content": sys_prompt })];

    for m in db.list_messages(8) {
        if m.text.trim().is_empty() {
            continue;
        }
        messages.push(json!({ "role": m.role, "content": m.text }));
    }

    // Current user turn (supports vision when using cloud + image_data)
    if let (Some(img), false) = (&image_data, use_local) {
        messages.push(json!({
            "role": "user",
            "content": [
                { "type": "text", "text": if user_text.trim().is_empty() { "给你看张图" } else { user_text } },
                { "type": "image_url", "image_url": { "url": img } }
            ]
        }));
    } else {
        let content = if image_data.is_some() && use_local {
            format!("{}\n（注：对方附带了一张图片，当前纯离线文本模型暂看不到画面，可自然提醒切换云端模式看图）", user_text)
        } else {
            user_text.to_string()
        };
        messages.push(json!({ "role": "user", "content": content }));
    }

    // Save user bubble first
    let _ = db.add_message(
        "user",
        if user_text.trim().is_empty() { "（发了一张图片）" } else { user_text },
        image_data.clone(),
    );

    let enable_tools = !use_local || matches!(local_status.model_tier.as_str(), "8B" | "7B" | "LOCAL");
    let tools = if enable_tools {
        build_tools_schema(can_draw)
    } else {
        Vec::new()
    };

    let weather_re = Regex::new(r"天气|多少度|几度|气温|冷不冷|热不热|带伞|下雨|下雪|降水|预报").unwrap();
    let ungrounded_weather_re = Regex::new(r"\d+(?:\.\d+)?\s*(?:度|℃|°C)|晴转|小雨|中雨|大雨|阵雨|多云转").unwrap();
    let is_weather_ask = weather_re.is_match(user_text);

    let mut called_tools: Vec<String> = Vec::new();
    let mut generated_drawing: Option<String> = None;
    let mut weather_guard_retried = false;

    for step in 0..4 {
        let mut req_body = json!({
            "model": model_name,
            "messages": messages,
            "temperature": if use_local { 0.35 } else { 0.65 },
            "max_tokens": 420
        });
        if !tools.is_empty() && step < 3 {
            req_body["tools"] = json!(tools);
        }

        let mut req = client
            .post(&endpoint_url)
            .json(&req_body)
            .timeout(std::time::Duration::from_secs(55));
        if !use_local {
            req = req.bearer_auth(&settings.api_key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("模型请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(format!("模型服务返回 {status}: {}", &err_txt.chars().take(140).collect::<String>()));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("响应解析失败: {e}"))?;

        let msg = &body["choices"][0]["message"];
        let raw_content = msg["content"].as_str().unwrap_or("").trim().to_string();

        // Collect structured OpenAI tool_calls OR fallback Qwen <tool_call> tags
        let mut calls: Vec<(String, String, Value)> = Vec::new();
        if let Some(arr) = msg["tool_calls"].as_array() {
            for (idx, tc) in arr.iter().enumerate() {
                let id = tc["id"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("tc_{idx}"));
                if let Some(fname) = tc["function"]["name"].as_str() {
                    let arg_val = match &tc["function"]["arguments"] {
                        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({})),
                        v @ Value::Object(_) => v.clone(),
                        _ => json!({}),
                    };
                    calls.push((id, fname.to_string(), arg_val));
                }
            }
        }
        if calls.is_empty() && raw_content.contains("<tool_call>") {
            calls = parse_inline_tool_calls(&raw_content);
        }

        if calls.is_empty() {
            let called_weather = called_tools.iter().any(|t| t == "companion_get_weather");
            if is_weather_ask
                && enable_tools
                && !called_weather
                && !weather_guard_retried
                && step < 3
                && ungrounded_weather_re.is_match(&raw_content)
            {
                weather_guard_retried = true;
                messages.push(json!({ "role": "assistant", "content": raw_content }));
                messages.push(json!({
                    "role": "user",
                    "content": "【系统校验】你刚才没调用 companion_get_weather 就报了温度。请立刻调用 companion_get_weather 仅查本轮城市，拿到真实天气后再自然回复。"
                }));
                continue;
            }

            let cleaned = sanitize_reply(&raw_content, &settings.partner_name);
            let final_reply = if cleaned.is_empty() || cleaned == "[NO_REPLY]" {
                "嗯，我在听呢".to_string()
            } else {
                cleaned
            };
            let _ = db.add_message("assistant", &final_reply, generated_drawing.clone());

            return Ok(ChatOutcome {
                reply: final_reply,
                model_used: if use_local {
                    format!("local:{}", if local_status.model_tier.is_empty() { "GGUF" } else { &local_status.model_tier })
                } else {
                    format!("cloud:{}", settings.cloud_model)
                },
                tool_calls: called_tools,
                drawing_image: generated_drawing,
                latency_ms: started.elapsed().as_millis(),
            });
        }

        // Execute tool calls
        messages.push(msg.clone());
        for (call_id, fname, args) in calls {
            called_tools.push(fname.clone());
            let result_val = match fname.as_str() {
                "companion_get_weather" => {
                    let city = args["city"].as_str().unwrap_or("");
                    let hint = args["dateHint"].as_str().or(args["date"].as_str()).unwrap_or("今天");
                    fetch_free_weather(client, city, hint).await
                }
                "companion_recall_events" => {
                    let q = args["query"].as_str().unwrap_or("");
                    json!({ "events": db.search_events(q, 10) })
                }
                "companion_stage_event_update" => {
                    let title = args["title"].as_str().unwrap_or("未命名事项");
                    let phase = args["phase"].as_str().unwrap_or("active");
                    let facts = args.get("facts").cloned().unwrap_or_else(|| json!({}));
                    let quote = args["quote"].as_str().unwrap_or(user_text);
                    match db.upsert_event(None, title, phase, &facts, quote) {
                        Ok(ev) => json!({ "ok": true, "saved": ev }),
                        Err(e) => json!({ "ok": false, "error": e }),
                    }
                }
                "companion_forget_memory" => {
                    let kw = args["keyword"].as_str().unwrap_or("");
                    let hits = db.search_events(kw, 5);
                    let mut count = 0;
                    for h in hits {
                        if db.delete_event(&h.event_id).is_ok() {
                            count += 1;
                        }
                    }
                    json!({ "ok": true, "deletedCount": count })
                }
                "companion_stage_shared_project" => {
                    let name = args["name"].as_str().unwrap_or("共同企划");
                    let stage = args["stage"].as_str().unwrap_or("进行中");
                    let notes = args["notes"].as_str().unwrap_or("");
                    match db.upsert_project(name, stage, notes) {
                        Ok(p) => json!({ "ok": true, "project": p }),
                        Err(e) => json!({ "ok": false, "error": e }),
                    }
                }
                "companion_submit_drawing" => {
                    if !can_draw {
                        json!({ "ok": false, "error": "当前离线模式未开启生图 Key" })
                    } else {
                        let prompt = args["prompt"].as_str().unwrap_or(user_text);
                        match execute_cloud_drawing(client, &settings, prompt).await {
                            Ok(data_url) => {
                                let _ = db.add_drawing(prompt, &data_url);
                                generated_drawing = Some(data_url);
                                json!({ "ok": true, "status": "generated" })
                            }
                            Err(e) => json!({ "ok": false, "error": e }),
                        }
                    }
                }
                _ => json!({ "ok": false, "error": "unknown tool" }),
            };

            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": result_val.to_string()
            }));
        }
    }

    let fallback = "刚走神了一下，你再说一遍？".to_string();
    let _ = db.add_message("assistant", &fallback, generated_drawing.clone());
    Ok(ChatOutcome {
        reply: fallback,
        model_used: model_name,
        tool_calls: called_tools,
        drawing_image: generated_drawing,
        latency_ms: started.elapsed().as_millis(),
    })
}
