# 小库原生桌面版（Windows `.exe` & macOS `.dmg` · Tauri 2）

零 Node.js 依赖、安装包不足 10 MB 的极简原生桌面虚拟朋友应用：
- **纯本地离线优先**：支持直接导入或自动识别 `Qwen3-8B` / `Qwen2.5-7B` / `Qwen2.5-3B` `.gguf` 模型（Qwen3 自动关闭 `<think>` 思考链）。
- **零 API 免费工具**：内置本地 SQLite 长期记忆（记事/遗忘/共创项目/导出导入记忆胶囊）、三段式脾气回怼、免费免 Key 查天气（Open-Meteo）。
- **可选云端扩展（BYOK）**：无 Key 时自动隐藏画图并自然告知；用户可在右上角 `···` 填入自己的 API Key 解锁云端看图与画图。
- **系统托盘常驻**：关闭窗口自动收进托盘，保持本地模型热启动不掉显存。

## 本地编译 Windows 安装包（`.exe`）
```powershell
npm install
npm run build:win
```
产物位于 `src-tauri/target/release/bundle/nsis/Xiaoku_1.0.0_x64-setup.exe`。

## 自动编译 macOS 安装包（`.dmg`）
推送到 GitHub 后，运行 `.github/workflows/build-xiaoku-native.yml` 即可同时产出：
- Windows `x64-setup.exe`
- macOS Apple Silicon (`aarch64`) `.dmg`
- macOS Intel (`x86_64`) `.dmg`
