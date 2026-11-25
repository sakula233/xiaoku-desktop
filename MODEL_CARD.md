# 小库离线伴侣模型（Qwen3-8B / Qwen2.5-7B · Q4_K_M GGUF）

> **开源基座声明（Apache-2.0）**：本微调权重分别基于 Alibaba Qwen 团队开源的 `Qwen3-8B` 与 `Qwen2.5-7B-Instruct`（Apache-2.0 许可）进行 LoRA SFT + 平衡 DPO 偏好对齐，所有训练数据均经过脱敏与零隐私泄漏审计。

## 推荐硬件档位

| 模型文件 | 体积 | 推荐配置 | 特点 |
| :--- | :--- | :--- | :--- |
| `xiaoku-qwen3-8b-q4_k_m.gguf` | ~5.03 GB | 8GB+ 独显（Vulkan/CUDA）或 16GB+ Apple Silicon Mac | 旗舰离线主脑（需关闭 `<think>` 快答模式），支持工具调用、三段脾气回怼与精准 `[NO_REPLY]` |
| `xiaoku-qwen2.5-7b-q4_k_m.gguf` | ~4.68 GB | 8GB 独显或 16GB Mac | 显存余量更充裕，原生 ChatML 工具调用稳健 |
| `xiaoku-qwen2.5-3b-q4_k_m.gguf` | ~1.80 GB | 核显轻薄本 / 纯 CPU / iPhone | 轻量快答保底档（基于 Qwen2.5-3B Research 协议，仅限个人学习研究） |

## 使用方式

### 1. 搭配「小库原生桌面版」（Windows `.exe` / macOS `.dmg`）
1. 下载对应平台安装包并打开，点击顶部提示栏 **「选择 .gguf」** 选中模型文件；
2. 零 API 费用即可拥有：**本地离线聊天 + 长期事件记忆 + 免费实时查天气 + 三段脾气互动 + 自由改名**。

### 2. 搭配 Ollama / LM Studio
```bash
# Qwen3-8B（关闭思考链快答）
ollama create xiaoku-8b -f Modelfile-Qwen3-8B
ollama run xiaoku-8b
```
- 若使用 `llama-server` 直接启动 Qwen3-8B：
  ```bash
  llama-server -m xiaoku-qwen3-8b-q4_k_m.gguf -ngl 99 -c 4096 --jinja --reasoning off --reasoning-budget 0
  ```
