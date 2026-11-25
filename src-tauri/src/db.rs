use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub partner_name: String,
    pub brain_mode: String, // "local-only" | "hybrid" | "cloud-only"
    pub custom_model_path: String,
    pub api_base_url: String,
    pub api_key: String,
    pub cloud_model: String,
    pub image_model: String,
    pub theme: String, // "system" | "light" | "dark"
    pub daily_summary: String,
    pub daily_date: String,
    pub mood: String,
    pub rage: i64,
    pub streak_provoke: i64,
    pub last_provoke_at: i64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            partner_name: "小库".to_string(),
            brain_mode: "local-only".to_string(),
            custom_model_path: String::new(),
            api_base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key: String::new(),
            cloud_model: "google/gemini-4-hydrogen".to_string(),
            image_model: "google/gemini-3-pro-image".to_string(),
            theme: "system".to_string(),
            daily_summary: "刚整理完昨晚的雨声歌单，顺手泡了杯热茶。".to_string(),
            daily_date: Utc::now().format("%Y-%m-%d").to_string(),
            mood: "平淡".to_string(),
            rage: 0,
            streak_provoke: 0,
            last_provoke_at: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub image_data: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventItem {
    pub event_id: String,
    pub title: String,
    pub phase: String,
    pub facts: Value,
    pub quote: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectItem {
    pub project_id: String,
    pub name: String,
    pub stage: String,
    pub notes: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawingItem {
    pub id: String,
    pub prompt: String,
    pub image_data: String,
    pub at: i64,
}

pub struct CompanionDb {
    conn: Mutex<Connection>,
    pub _db_path: PathBuf,
}

impl CompanionDb {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create data dir failed: {e}"))?;
        }
        let conn = Connection::open(db_path).map_err(|e| format!("open sqlite failed: {e}"))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS kv_settings (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                text TEXT NOT NULL,
                image_data TEXT,
                at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_at ON messages(at);
            CREATE TABLE IF NOT EXISTS events (
                event_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                phase TEXT NOT NULL,
                facts_json TEXT NOT NULL,
                quote TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS projects (
                project_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                stage TEXT NOT NULL,
                notes TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS drawings (
                id TEXT PRIMARY KEY,
                prompt TEXT NOT NULL,
                image_data TEXT NOT NULL,
                at INTEGER NOT NULL
            );
            ",
        )
        .map_err(|e| format!("init schema failed: {e}"))?;

        Ok(Self {
            conn: Mutex::new(conn),
            _db_path: db_path.to_path_buf(),
        })
    }

    pub fn load_settings(&self) -> AppSettings {
        let conn = self.conn.lock().unwrap();
        let mut s = AppSettings::default();
        if let Ok(mut stmt) = conn.prepare("SELECT k, v FROM kv_settings") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for pair in rows.flatten() {
                    match pair.0.as_str() {
                        "partnerName" => s.partner_name = pair.1,
                        "brainMode" => s.brain_mode = pair.1,
                        "customModelPath" => s.custom_model_path = pair.1,
                        "apiBaseUrl" => s.api_base_url = pair.1,
                        "apiKey" => s.api_key = pair.1,
                        "cloudModel" => {
                            s.cloud_model = if pair.1 == "google/gemini-2.5-flash" {
                                "google/gemini-4-hydrogen".to_string()
                            } else {
                                pair.1
                            };
                        }
                        "imageModel" => {
                            s.image_model = if pair.1 == "google/gemini-2.5-flash-image-preview" {
                                "google/gemini-3-pro-image".to_string()
                            } else {
                                pair.1
                            };
                        }
                        "theme" => s.theme = pair.1,
                        "dailySummary" => s.daily_summary = pair.1,
                        "dailyDate" => s.daily_date = pair.1,
                        "mood" => s.mood = pair.1,
                        "rage" => s.rage = pair.1.parse().unwrap_or(0),
                        "streakProvoke" => s.streak_provoke = pair.1.parse().unwrap_or(0),
                        "lastProvokeAt" => s.last_provoke_at = pair.1.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
        }
        s
    }

    pub fn save_settings(&self, s: &AppSettings) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let pairs: [(&str, String); 14] = [
            ("partnerName", s.partner_name.clone()),
            ("brainMode", s.brain_mode.clone()),
            ("customModelPath", s.custom_model_path.clone()),
            ("apiBaseUrl", s.api_base_url.clone()),
            ("apiKey", s.api_key.clone()),
            ("cloudModel", s.cloud_model.clone()),
            ("imageModel", s.image_model.clone()),
            ("theme", s.theme.clone()),
            ("dailySummary", s.daily_summary.clone()),
            ("dailyDate", s.daily_date.clone()),
            ("mood", s.mood.clone()),
            ("rage", s.rage.to_string()),
            ("streakProvoke", s.streak_provoke.to_string()),
            ("lastProvokeAt", s.last_provoke_at.to_string()),
        ];
        for (k, v) in pairs {
            conn.execute(
                "INSERT INTO kv_settings (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                params![k, v],
            )
            .map_err(|e| format!("save setting {k} failed: {e}"))?;
        }
        Ok(())
    }

    pub fn add_message(&self, role: &str, text: &str, image_data: Option<String>) -> Result<ChatMessage, String> {
        let msg = ChatMessage {
            id: format!("m-{}", &Uuid::new_v4().to_string()[..10]),
            role: role.to_string(),
            text: text.to_string(),
            image_data,
            at: Utc::now().timestamp_millis(),
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (id, role, text, image_data, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![msg.id, msg.role, msg.text, msg.image_data, msg.at],
        )
        .map_err(|e| format!("insert message failed: {e}"))?;
        Ok(msg)
    }

    pub fn list_messages(&self, limit: usize) -> Vec<ChatMessage> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, role, text, image_data, at FROM messages ORDER BY at DESC LIMIT ?1",
        ) {
            if let Ok(rows) = stmt.query_map(params![limit as i64], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    text: row.get(2)?,
                    image_data: row.get(3)?,
                    at: row.get(4)?,
                })
            }) {
                for item in rows.flatten() {
                    out.push(item);
                }
            }
        }
        out.reverse();
        out
    }

    pub fn clear_messages(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM messages", [])
            .map_err(|e| format!("clear messages failed: {e}"))?;
        Ok(())
    }

    pub fn upsert_event(
        &self,
        event_id: Option<&str>,
        title: &str,
        phase: &str,
        facts: &Value,
        quote: &str,
    ) -> Result<EventItem, String> {
        let id = event_id
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("ev-{}", &Uuid::new_v4().to_string()[..8]));
        let now = Utc::now().timestamp_millis();
        let facts_json = serde_json::to_string(facts).unwrap_or_else(|_| "{}".to_string());
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (event_id, title, phase, facts_json, quote, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(event_id) DO UPDATE SET
               title = excluded.title,
               phase = excluded.phase,
               facts_json = excluded.facts_json,
               quote = excluded.quote,
               updated_at = excluded.updated_at",
            params![id, title, phase, facts_json, quote, now],
        )
        .map_err(|e| format!("upsert event failed: {e}"))?;
        Ok(EventItem {
            event_id: id,
            title: title.to_string(),
            phase: phase.to_string(),
            facts: facts.clone(),
            quote: quote.to_string(),
            updated_at: now,
        })
    }

    pub fn search_events(&self, query: &str, limit: usize) -> Vec<EventItem> {
        let conn = self.conn.lock().unwrap();
        let mut all = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT event_id, title, phase, facts_json, quote, updated_at FROM events ORDER BY updated_at DESC LIMIT 100",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                let facts_raw: String = row.get(3)?;
                let facts: Value = serde_json::from_str(&facts_raw).unwrap_or_else(|_| json!({}));
                Ok(EventItem {
                    event_id: row.get(0)?,
                    title: row.get(1)?,
                    phase: row.get(2)?,
                    facts,
                    quote: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            }) {
                for ev in rows.flatten() {
                    all.push(ev);
                }
            }
        }
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return all.into_iter().take(limit).collect();
        }
        all.into_iter()
            .filter(|e| {
                e.title.to_lowercase().contains(&q)
                    || e.quote.to_lowercase().contains(&q)
                    || e.facts.to_string().to_lowercase().contains(&q)
            })
            .take(limit)
            .collect()
    }

    pub fn delete_event(&self, event_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM events WHERE event_id = ?1", params![event_id])
            .map_err(|e| format!("delete event failed: {e}"))?;
        Ok(())
    }

    pub fn upsert_project(&self, name: &str, stage: &str, notes: &str) -> Result<ProjectItem, String> {
        let now = Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT project_id FROM projects WHERE name = ?1 LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .ok();
        let id = existing.unwrap_or_else(|| format!("proj-{}", &Uuid::new_v4().to_string()[..8]));
        conn.execute(
            "INSERT INTO projects (project_id, name, stage, notes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id) DO UPDATE SET
               stage = excluded.stage,
               notes = excluded.notes,
               updated_at = excluded.updated_at",
            params![id, name, stage, notes, now],
        )
        .map_err(|e| format!("upsert project failed: {e}"))?;
        Ok(ProjectItem {
            project_id: id,
            name: name.to_string(),
            stage: stage.to_string(),
            notes: notes.to_string(),
            updated_at: now,
        })
    }

    pub fn list_projects(&self) -> Vec<ProjectItem> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT project_id, name, stage, notes, updated_at FROM projects ORDER BY updated_at DESC LIMIT 30",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok(ProjectItem {
                    project_id: row.get(0)?,
                    name: row.get(1)?,
                    stage: row.get(2)?,
                    notes: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            }) {
                for p in rows.flatten() {
                    out.push(p);
                }
            }
        }
        out
    }

    pub fn add_drawing(&self, prompt: &str, image_data: &str) -> Result<DrawingItem, String> {
        let item = DrawingItem {
            id: format!("draw-{}", &Uuid::new_v4().to_string()[..8]),
            prompt: prompt.to_string(),
            image_data: image_data.to_string(),
            at: Utc::now().timestamp_millis(),
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO drawings (id, prompt, image_data, at) VALUES (?1, ?2, ?3, ?4)",
            params![item.id, item.prompt, item.image_data, item.at],
        )
        .map_err(|e| format!("insert drawing failed: {e}"))?;
        Ok(item)
    }

    pub fn list_drawings(&self, limit: usize) -> Vec<DrawingItem> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, prompt, image_data, at FROM drawings ORDER BY at DESC LIMIT ?1",
        ) {
            if let Ok(rows) = stmt.query_map(params![limit as i64], |row| {
                Ok(DrawingItem {
                    id: row.get(0)?,
                    prompt: row.get(1)?,
                    image_data: row.get(2)?,
                    at: row.get(3)?,
                })
            }) {
                for d in rows.flatten() {
                    out.push(d);
                }
            }
        }
        out
    }

    pub fn export_capsule(&self) -> Value {
        let settings = self.load_settings();
        json!({
            "format": "xiaoku-native-capsule-v1",
            "exportedAt": Utc::now().to_rfc3339(),
            "partnerName": settings.partner_name,
            "dailySummary": settings.daily_summary,
            "events": self.search_events("", 200),
            "projects": self.list_projects(),
            "messages": self.list_messages(300),
        })
    }

    pub fn import_capsule(&self, payload: &Value) -> Result<(usize, usize), String> {
        let mut imported_events = 0usize;
        let mut imported_projects = 0usize;
        if let Some(name) = payload.get("partnerName").and_then(|v| v.as_str()) {
            let clean = name.trim();
            if !clean.is_empty() && clean.chars().count() <= 16 {
                let mut s = self.load_settings();
                s.partner_name = clean.to_string();
                let _ = self.save_settings(&s);
            }
        }
        if let Some(events) = payload.get("events").and_then(|v| v.as_array()) {
            for ev in events {
                let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
                if title.is_empty() {
                    continue;
                }
                let phase = ev.get("phase").and_then(|v| v.as_str()).unwrap_or("active");
                let facts = ev.get("facts").cloned().unwrap_or_else(|| json!({}));
                let quote = ev.get("quote").and_then(|v| v.as_str()).unwrap_or("");
                let eid = ev.get("eventId").and_then(|v| v.as_str());
                if self.upsert_event(eid, title, phase, &facts, quote).is_ok() {
                    imported_events += 1;
                }
            }
        }
        if let Some(projects) = payload.get("projects").and_then(|v| v.as_array()) {
            for p in projects {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
                if name.is_empty() {
                    continue;
                }
                let stage = p.get("stage").and_then(|v| v.as_str()).unwrap_or("进行中");
                let notes = p.get("notes").and_then(|v| v.as_str()).unwrap_or("");
                if self.upsert_project(name, stage, notes).is_ok() {
                    imported_projects += 1;
                }
            }
        }
        Ok((imported_events, imported_projects))
    }
}
