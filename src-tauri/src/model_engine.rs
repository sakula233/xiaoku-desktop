use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStatus {
    pub online: bool,
    pub endpoint: String,
    pub model_path: String,
    pub model_name: String,
    pub model_tier: String, // "8B" | "7B" | "3B" | "CUSTOM" | ""
    pub engine_found: bool,
    pub engine_path: String,
    pub managed_by_app: bool,
}

pub struct LocalModelManager {
    app_data_dir: PathBuf,
    resource_dir: Option<PathBuf>,
    child: Mutex<Option<Child>>,
    active_port: Mutex<u16>,
    active_model_path: Mutex<String>,
}

impl LocalModelManager {
    pub fn new(app_data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Self {
        Self {
            app_data_dir,
            resource_dir,
            child: Mutex::new(None),
            active_port: Mutex::new(11435),
            active_model_path: Mutex::new(String::new()),
        }
    }

    pub fn models_dir(&self) -> PathBuf {
        let p = self.app_data_dir.join("models");
        let _ = std::fs::create_dir_all(&p);
        p
    }

    pub fn detect_tier(path: &str) -> String {
        let lower = path.to_lowercase();
        if lower.contains("qwen3-8b") || lower.contains("8b") {
            "8B".to_string()
        } else if lower.contains("7b") {
            "7B".to_string()
        } else if lower.contains("3b") || lower.contains("4b") {
            "3B".to_string()
        } else if !lower.is_empty() {
            "LOCAL".to_string()
        } else {
            String::new()
        }
    }

    pub fn find_model_file(&self, custom_path: &str) -> Option<PathBuf> {
        if !custom_path.trim().is_empty() {
            let p = PathBuf::from(custom_path.trim());
            if p.is_file() {
                return Some(p);
            }
        }

        let mut search_dirs = vec![self.models_dir()];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                search_dirs.push(dir.join("models"));
                // Also support running straight from repo dev build
                let mut cur = dir.to_path_buf();
                for _ in 0..6 {
                    let dev_models = cur.join("data").join("local-model");
                    if dev_models.is_dir() {
                        search_dirs.push(dev_models);
                        break;
                    }
                    let cp_models = cur.join("companion-platform").join("data").join("local-model");
                    if cp_models.is_dir() {
                        search_dirs.push(cp_models);
                        break;
                    }
                    if !cur.pop() {
                        break;
                    }
                }
            }
        }

        let preferred = [
            "xiaoku-qwen3-8b-q4_k_m.gguf",
            "xiaoku-qwen2.5-7b-q4_k_m.gguf",
            "xiaoku-qwen2.5-3b-q4_k_m.gguf",
        ];
        for name in preferred {
            for dir in &search_dirs {
                if let Some(found) = find_file_recursive(dir, name, 2) {
                    return Some(found);
                }
            }
        }

        // Fallback: first .gguf found in any search dir (including 1-level subfolders)
        for dir in &search_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("gguf"))
                            .unwrap_or(false)
                    {
                        return Some(p);
                    }
                    if p.is_dir() {
                        if let Ok(sub) = std::fs::read_dir(&p) {
                            for se in sub.flatten() {
                                let sp = se.path();
                                if sp.is_file()
                                    && sp.extension()
                                        .and_then(|e| e.to_str())
                                        .map(|e| e.eq_ignore_ascii_case("gguf"))
                                        .unwrap_or(false)
                                {
                                    return Some(sp);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub fn find_llama_server(&self) -> Option<PathBuf> {
        let bin_name = if cfg!(target_os = "windows") {
            "llama-server.exe"
        } else {
            "llama-server"
        };

        let mut dirs = vec![
            self.app_data_dir.join("engine"),
            self.app_data_dir.join("models").join("llama-vulkan"),
        ];
        if let Some(res_dir) = &self.resource_dir {
            dirs.push(res_dir.join("engine"));
            dirs.push(res_dir.clone());
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                dirs.push(dir.to_path_buf());
                dirs.push(dir.join("engine"));
                dirs.push(dir.join("resources").join("engine"));
                let mut cur = dir.to_path_buf();
                for _ in 0..6 {
                    let dev_eng = cur.join("data").join("local-model").join("llama-vulkan");
                    if dev_eng.is_dir() {
                        dirs.push(dev_eng);
                        break;
                    }
                    let cp_eng = cur
                        .join("companion-platform")
                        .join("data")
                        .join("local-model")
                        .join("llama-vulkan");
                    if cp_eng.is_dir() {
                        dirs.push(cp_eng);
                        break;
                    }
                    if !cur.pop() {
                        break;
                    }
                }
            }
        }

        for dir in dirs {
            if let Some(found) = find_file_recursive(&dir, bin_name, 3) {
                return Some(found);
            }
        }

        // Check PATH (e.g. Homebrew llama.cpp on macOS)
        if let Ok(path_var) = std::env::var("PATH") {
            let sep = if cfg!(target_os = "windows") { ';' } else { ':' };
            for part in path_var.split(sep) {
                let candidate = PathBuf::from(part).join(bin_name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        if cfg!(target_os = "macos") {
            for brew in ["/opt/homebrew/bin/llama-server", "/usr/local/bin/llama-server"] {
                let p = PathBuf::from(brew);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    pub async fn probe_health(client: &reqwest::Client, port: u16) -> bool {
        let url = format!("http://127.0.0.1:{port}/health");
        match client
            .get(&url)
            .timeout(std::time::Duration::from_millis(750))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn ensure_started(
        &self,
        client: &reqwest::Client,
        custom_model_path: &str,
        force_restart: bool,
    ) -> LocalModelStatus {
        let model_opt = self.find_model_file(custom_model_path);
        let engine_opt = self.find_llama_server();
        let target_model_str = model_opt
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if force_restart {
            self.stop();
        }

        // Reuse existing healthy port (11435 external or app-managed port) when not forcing a model switch
        let cur_port = *self.active_port.lock().unwrap();
        let cur_model = self.active_model_path.lock().unwrap().clone();
        if !force_restart && (cur_model.is_empty() || cur_model == target_model_str) {
            if Self::probe_health(client, cur_port).await {
                return self.build_status(true, cur_port, &model_opt, &engine_opt);
            }
            if cur_port != 11435 && Self::probe_health(client, 11435).await {
                *self.active_port.lock().unwrap() = 11435;
                return self.build_status(true, 11435, &model_opt, &engine_opt);
            }
        }

        let (Some(model_path), Some(engine_path)) = (&model_opt, &engine_opt) else {
            return self.build_status(false, cur_port, &model_opt, &engine_opt);
        };

        // Choose 11435 if free, else 11436
        let spawn_port: u16 = if Self::probe_health(client, 11435).await {
            11436
        } else {
            11435
        };

        self.stop();
        let mut cmd = Command::new(engine_path);
        cmd.arg("-m")
            .arg(model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(spawn_port.to_string())
            .arg("-ngl")
            .arg("99")
            .arg("-c")
            .arg("4096")
            .arg("--jinja")
            .arg("--alias")
            .arg("xiaoku-local-brain")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if target_model_str.to_lowercase().contains("qwen3") {
            cmd.arg("--reasoning")
                .arg("off")
                .arg("--reasoning-budget")
                .arg("0");
        }
        if let Some(work_dir) = engine_path.parent() {
            cmd.current_dir(work_dir);
        }
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        if let Ok(child) = cmd.spawn() {
            *self.child.lock().unwrap() = Some(child);
            *self.active_port.lock().unwrap() = spawn_port;
            *self.active_model_path.lock().unwrap() = target_model_str;
            for _ in 0..45 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if Self::probe_health(client, spawn_port).await {
                    return self.build_status(true, spawn_port, &model_opt, &engine_opt);
                }
            }
        }

        self.build_status(false, spawn_port, &model_opt, &engine_opt)
    }

    pub async fn status(&self, client: &reqwest::Client, custom_model_path: &str) -> LocalModelStatus {
        let model_opt = self.find_model_file(custom_model_path);
        let engine_opt = self.find_llama_server();
        let cur_port = *self.active_port.lock().unwrap();
        let online = Self::probe_health(client, cur_port).await;
        self.build_status(online, cur_port, &model_opt, &engine_opt)
    }

    fn build_status(
        &self,
        online: bool,
        port: u16,
        model_opt: &Option<PathBuf>,
        engine_opt: &Option<PathBuf>,
    ) -> LocalModelStatus {
        let model_path = model_opt
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let model_name = model_opt
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let model_tier = Self::detect_tier(&model_name);
        let engine_path = engine_opt
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let managed_by_app = self.child.lock().unwrap().is_some();

        LocalModelStatus {
            online,
            endpoint: format!("http://127.0.0.1:{port}/v1"),
            model_path,
            model_name,
            model_tier,
            engine_found: engine_opt.is_some(),
            engine_path,
            managed_by_app,
        }
    }

    pub fn stop(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

impl Drop for LocalModelManager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn find_file_recursive(dir: &Path, filename: &str, max_depth: usize) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.eq_ignore_ascii_case(filename))
                    .unwrap_or(false)
            {
                return Some(p);
            }
            if max_depth > 0 && p.is_dir() {
                if let Some(nested) = find_file_recursive(&p, filename, max_depth - 1) {
                    return Some(nested);
                }
            }
        }
    }
    None
}
