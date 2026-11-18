# 更新日志 Changelog

所有显著变更都会记录在本文件。格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循语义化版本。

## v0.2.2 (2026-11-18)

### 修复

- **macOS 安装包改为通用二进制（Universal）**：一个 `universal.dmg` 同时包含 Intel（x86_64）与 Apple Silicon（aarch64），Intel Mac 与 M 系列芯片均可运行；此前只产出 arm64 包，Intel Mac 无法运行
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
