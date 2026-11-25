#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$AppRoot  = Split-Path -Parent $PSScriptRoot
$PlatRoot = Split-Path -Parent (Split-Path -Parent $AppRoot)
$RelDir   = Join-Path $AppRoot 'src-tauri\target\release'
$DistBuild= Join-Path $AppRoot 'dist-build'
$NsisExe  = Get-ChildItem $DistBuild,(Join-Path $RelDir 'bundle\nsis') -Filter '*setup*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
$MainExe  = if (Test-Path -LiteralPath (Join-Path $DistBuild 'xiaoku-native.exe')) { Join-Path $DistBuild 'xiaoku-native.exe' } else { Join-Path $RelDir 'xiaoku-native.exe' }
$Vulkan   = Join-Path $PlatRoot 'data\local-model\llama-vulkan'
$OutDir   = Join-Path $AppRoot 'dist-share\Xiaoku-Win64-Release'

if (-not (Test-Path -LiteralPath $MainExe)) {
    throw "请先运行 npm run build:win 编译原生程序（缺少 $MainExe）"
}

if (Test-Path -LiteralPath $OutDir) { Remove-Item -LiteralPath $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

# 1. 复制原生安装程序 (.exe)
if ($NsisExe) {
    Copy-Item -LiteralPath $NsisExe.FullName -Destination (Join-Path $OutDir $NsisExe.Name) -Force
}

# 2. 组装免安装绿色原生版（零私人数据、零 SQLite、零 .env Key）
$Portable = Join-Path $OutDir 'Xiaoku-Portable-Win64'
$EngDir   = Join-Path $Portable 'engine'
$ModDir   = Join-Path $Portable 'models'
New-Item -ItemType Directory -Path $EngDir, $ModDir -Force | Out-Null

Copy-Item -LiteralPath $MainExe -Destination (Join-Path $Portable 'Xiaoku.exe') -Force
if (Test-Path -LiteralPath $Vulkan) {
    Copy-Item -Path (Join-Path $Vulkan '*') -Destination $EngDir -Recurse -Force
}

@"
把下载好的小库离线模型放进这个文件夹即可（支持三档，优先自动识别最高档）：
  1. xiaoku-qwen3-8b-q4_k_m.gguf   （约 5.03 GB，推荐 8GB+ 独显）
  2. xiaoku-qwen2.5-7b-q4_k_m.gguf （约 4.68 GB，8GB 独显更宽松）
  3. xiaoku-qwen2.5-3b-q4_k_m.gguf （约 1.80 GB，核显/轻薄本保底）

放好后直接双击上一级目录的 Xiaoku.exe 即可离线开聊（零安装、零 API 费用）。
"@ | Set-Content -LiteralPath (Join-Path $ModDir '把GGUF模型放这里.txt') -Encoding UTF8

Copy-Item -LiteralPath (Join-Path $AppRoot 'MODEL_CARD.md') -Destination (Join-Path $OutDir '使用说明.md') -Force

# 3. 写入 Ollama Modelfile 模板
@"
FROM ./xiaoku-qwen3-8b-q4_k_m.gguf
PARAMETER temperature 0.35
PARAMETER top_p 0.8
PARAMETER stop "<|im_end|>"
SYSTEM """你是「小库」，用户的虚拟朋友（银白短发、粉紫眼、粉色发夹）。像熟人朋友随口接话，句子短促自然，严禁客服套话和编号列表；对方挑衅开骂时不讨好直接回怼。"""
"@ | Set-Content -LiteralPath (Join-Path $OutDir 'Modelfile-Qwen3-8B') -Encoding UTF8

@"
FROM ./xiaoku-qwen2.5-7b-q4_k_m.gguf
PARAMETER temperature 0.35
PARAMETER stop "<|im_end|>"
SYSTEM """你是「小库」，用户的虚拟朋友（银白短发、粉紫眼、粉色发夹）。像熟人朋友随口接话，句子短促自然，严禁客服套话和编号列表；对方挑衅开骂时不讨好直接回怼。"""
"@ | Set-Content -LiteralPath (Join-Path $OutDir 'Modelfile-Qwen2.5-7B') -Encoding UTF8

Write-Host "✅ 零隐私分发包已生成：$OutDir" -ForegroundColor Green
Get-ChildItem $OutDir | Select-Object Name, Length
