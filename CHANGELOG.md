# 更新日志 Changelog

所有显著变更都会记录在本文件。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循语义化版本。

## v0.2.8 (2026-11-18)

### 修复

- **macOS 看不到同步过来的 SSH 密码**：本地 `secrets.json` 缺失时，打开 SSH 页会自动从仓库的 `encrypted/secrets.json.enc` 用主密码解密还原（权限 600）
- **macOS 无法测试连通性**：常规主机改为直接 `user@host -p 端口` 连接，不再依赖 `~/.ssh/config` 别名解析（macOS 未装 assh 也能测）；密码认证改用内置 PTY 自动输入，**无需安装 sshpass**
- **macOS 未安装 assh 时无法生成 `~/.ssh/config`**：新增内置基础生成器，直接从 `assh.yml` 生成标准 config（主机/端口/用户/密钥/模板默认值/StrictHostKeyChecking）；跳板机（gateways）仍需安装 assh
- 内嵌终端的密码主机同样改为内置 PTY 自动输入，不再依赖 sshpass

## v0.2.7 (2026-11-18)

### 说明

- 自动升级链路验证版本：功能与 v0.2.6 相同，用于验证「检查更新 → 立即升级 → 自动安装重启」完整流程

## v0.2.6 (2026-11-18)

### 新增

- **应用内自动升级**：检查到新版本后点「⇣ 立即升级」，应用会下载对应平台安装包并自动安装、重启
  - macOS：解压 zip，后台脚本等待应用退出后替换 `/Applications/nexus-workbench.app`、清除隔离标记并重新打开
  - Windows：以 `/S` 静默运行安装程序后退出
  - Linux：替换当前 AppImage 并重启（deb 方式会打开安装包由系统安装）
- 升级下载显示百分比进度条

### 修复

- **SSH 单独恢复很慢**：改为 blobless 部分克隆 + `--depth=1` 浅拉取，只惰性下载 `assh.yml` 单个文件，不再把整个数据仓库（数百 MB）重新拉一遍

### 说明

- 自动升级通过 HTTPS 匿名下载 Release 资源，**要求代码仓库保持公开**；若改为私有仓库，请使用「前往下载」按钮手动下载

## v0.2.5 (2026-11-18)

### 修复

- **macOS / Windows 恢复模式 SSH 主机列表为空**：`~/.ssh/assh.yml` 与仓库内 `assh.yml` 是两个独立文件（Linux 为软链接所以正常）。现在恢复/拉取/强制恢复后会自动把仓库版本同步到 `~/.ssh`，推送前再把本机编辑同步回仓库
- 旧版本已执行过恢复、但本地仍是「引导占位 assh.yml」的情况：SSH 页面加载时会自动用仓库中的完整配置替换
- `~/.ssh/assh.yml` 缺失时（如首次恢复后未重启）自动从仓库补齐

## v0.2.4 (2026-11-18)

### 新增

- macOS 额外提供 `nexus-workbench_<版本>_macos-universal.zip`：内含 `.app` 与 `install-macos.command` 一键安装脚本（自动解除 Gatekeeper 隔离、安装到 /Applications 并启动）

### 说明

- macOS 未做 Apple 公证，从浏览器下载后首次运行会被 Gatekeeper 拦截并提示「文件已损坏」（并非真的损坏，二进制已校验为 universal：x86_64 minos 10.13 / arm64 minos 11.0）
  - 推荐：下载 zip → 解压 → 双击 `install-macos.command`（如被拦截，右键 →「打开」）
  - 或下载 dmg 后执行：`xattr -cr /Applications/nexus-workbench.app` 再打开
  - 或对 dmg 本身：`xattr -cr ~/Downloads/nexus-workbench_<版本>_universal.dmg`

## v0.2.3 (2026-11-18)

### 修复

- **macOS 安装包改为通用二进制（Universal）**：一个 `universal.dmg` 同时包含 Intel（x86_64）与 Apple Silicon（aarch64），Intel Mac 与 M 系列芯片均可运行；此前只产出 arm64 包，Intel Mac 无法运行
- macOS 构建 runner 修正为 `macos-latest`（`macos-14` 镜像已下线，导致 job 被取消）
- macOS 安装包启用 ad-hoc 签名，减少「文件已损坏」误报；未公证的应用首次打开请右键 →「打开」，或执行 `xattr -cr /Applications/nexus-workbench.app`

## v0.2.0 (2026-11-18)

### 新增

- 首次使用引导：恢复模式 / 全新模式，全程实时进度条（加密 / 拉取 / 上传对象百分比）、已用时间、成功后 5 秒倒计时进入主界面
- 网络自动重试：连接验证、拉取、推送失败自动重试 3 次，进度条显示「第 N 次重试」
- Git 地址严格校验：仅接受 `git@host:owner/repo.git`，其他格式输入时实时拒绝（引导 / 诊断 / 保存配置三层拦截）
- 设置页「系统与升级」区块：操作系统类型、系统版本、登录账户、应用版本号，一键「检查更新」（私有库走本机 SSH，无需 token），有新版本一键跳转 Releases 下载
- 设置页「网络加速推荐」区块
- SSH 页「⇡ 推送到仓库」：只加密推送 assh.yml + 主机密码库，不动笔记 / AI 会话
- SSH 页「⇣ 从云端恢复」适配加密：取回密文解密还原，兼容旧明文仓库
- 主机密码库加密同步：`secrets.json` 随仓库加密为 `encrypted/secrets.json.enc`，恢复后可直接查看密码
- 升级检查支持读取更新日志（CHANGELOG.md 对应版本段落直接显示在应用内）

### 修复

- 打开设置页不再误启动 Electerm（探测不再执行 `--version`）
- 全新模式对 GitHub 空树仓库的推送被拒（fetch first）问题：已验证云端为空后强制覆盖
- `assh.yml` 缺失时恢复流程自身断网的问题：自动生成最小传输引导配置
- 主密码填错时的误报：明确提示「主密码不正确」并聚焦密码框，修正后重试无需重新下载

## v0.1.0 (2026-11-16)

- 首个内部版本
- SSH 管理（assh.yml 托管、连通性测试、密码本地保管、Electerm 集成）
- 内嵌多标签终端（真实 PTY，切页不断开）
- AI 会话同步（ZCode / Claude Code / Cursor / Trae / DSH）
- 笔记同步（Markdown / 富文本双模式、有道云拉取、清空后推送）
- 统一同步底座（一键推送 / 拉取 / 强制恢复、crontab 定时、应用内定时器兜底）
- 全量内容加密同步（AES-256-GCM，10 万轮派生）
- 5 套主题 + 工作台名称自定义
