#!/usr/bin/env bash
# =============================================================================
# ai-session-sync — 把 AI IDE / Coding Agent 的本地会话数据同步到 git 私有仓库
#
# 覆盖: ZCode / Claude Code / Cursor / Trae(豆包系AI IDE) / 豆包(Doubao 桌面版) / WorkBuddy
#       豆包、DeepSeek 等纯云端应用会话在厂商服务器上，登录即恢复，无需本地同步
#
# 用法:
#   sync.sh init [--remote <git-url>]   初始化仓库(可顺便绑定远程私有仓库)
#   sync.sh push                        快照 -> 收集 -> commit -> push
#   sync.sh pull [--force] [--mirror]   拉取远程并恢复到本机各工具目录
#   sync.sh status                      查看各工具检测结果 / 仓库状态
#   sync.sh squash                      压缩历史(仓库过大时用)
#   sync.sh cron install|remove|status  管理每小时自动 push 的 crontab 任务
# =============================================================================
set -euo pipefail

REPO_DIR="${AI_SYNC_REPO:-$HOME/ai-session-sync}"
CONF="$REPO_DIR/tools.conf"

# ------------------- 可调开关(可被 tools.conf 覆盖) -------------------
SYNC_WORKSPACES=1   # 同步 ~/.zcode/workspace (每个会话独立工作区的项目文件)
SYNC_ROLLOUT=1      # 同步 ZCode 原始模型 I/O 日志 (jsonl 文本, git 增量很小)
SYNC_ARTIFACTS=0    # 同步 ZCode 会话产物 artifacts (可能含大文件, 默认关)
SYNC_DSH_CREDENTIALS=0  # 同步 dsh 凭据(.credentials.yaml / api-key), 默认关!
TOOL_ZCODE=1
TOOL_CLAUDE=1
TOOL_CURSOR=1
TOOL_TRAE=1
TOOL_DSH=1
TOOL_DOUBAO=1
[ -f "$CONF" ] && . "$CONF" || true

# ------------------- 平台检测与数据路径 -------------------
# 各 AI 工具的配置目录在不同系统下位置不同：
#   ZCode / Claude Code / dsh : 三平台都在用户主目录下的隐藏目录
#   Cursor / Trae             : Linux 在 ~/.config，macOS 在 Application Support，Windows 在 %APPDATA%
case "$(uname -s 2>/dev/null)" in
  Darwin)                PLATFORM="macos"   ;;
  Linux)                 PLATFORM="linux"   ;;
  MINGW*|MSYS*|CYGWIN*)  PLATFORM="windows" ;;
  *)                     PLATFORM="unknown" ;;
esac

# 主目录（Windows Git Bash 下 $HOME 通常已正确指向 C:\Users\<name>）
HOME_DIR="${HOME:-$USERPROFILE}"

# ZCode / Claude Code / DeepSeek Harness：三平台统一位于主目录
ZCODE_HOME="${ZCODE_HOME:-$HOME_DIR/.zcode}"
CLAUDE_HOME="${CLAUDE_HOME:-$HOME_DIR/.claude}"
DSH_HOME="${DSH_HOME:-$HOME_DIR/.dsh}"

# 豆包桌面版(Electron): Windows %APPDATA%/Doubao, macOS ~/Library/Application Support/Doubao, Linux ~/.config/Doubao
case "$PLATFORM" in
  macos)    DOUBAO_HOME="${DOUBAO_HOME:-$HOME_DIR/Library/Application Support/Doubao}" ;;
  windows)  APP_DATA="${APPDATA:-$HOME_DIR/AppData/Roaming}"; DOUBAO_HOME="${DOUBAO_HOME:-$APP_DATA/Doubao}" ;;
  *)        XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME_DIR/.config}"; DOUBAO_HOME="${DOUBAO_HOME:-$XDG_CONFIG/Doubao}" ;;
esac

# WorkBuddy(Electron): 目录名大小写不一, 自动在常见位置探测; 可用 WORKBUDDY_HOME 强制指定
if [ -z "${WORKBUDDY_HOME:-}" ]; then
  for _wb in "$APP_DATA/WorkBuddy" "$APP_DATA/workbuddy" "$APP_DATA/WorkBuddy AI" \
             "$XDG_CONFIG/WorkBuddy" "$XDG_CONFIG/workbuddy" \
             "$HOME_DIR/Library/Application Support/WorkBuddy" "$HOME_DIR/Library/Application Support/workbuddy"; do
    [ -d "$_wb" ] && WORKBUDDY_HOME="$_wb" && break
  done
fi
WORKBUDDY_HOME="${WORKBUDDY_HOME:-$XDG_CONFIG/WorkBuddy}"

# Cursor / Trae：按平台选择数据根目录
case "$PLATFORM" in
  macos)
    CURSOR_HOME="${CURSOR_HOME:-$HOME_DIR/Library/Application Support/Cursor}"
    TRAE_HOME="${TRAE_HOME:-$HOME_DIR/Library/Application Support/Trae}"
    ;;
  windows)
    APP_DATA="${APPDATA:-$HOME_DIR/AppData/Roaming}"
    CURSOR_HOME="${CURSOR_HOME:-$APP_DATA/Cursor}"
    TRAE_HOME="${TRAE_HOME:-$APP_DATA/Trae}"
    ;;
  *)
    # Linux 及其他类 Unix：遵循 XDG 规范
    XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME_DIR/.config}"
    CURSOR_HOME="${CURSOR_HOME:-$XDG_CONFIG/Cursor}"
    TRAE_HOME="${TRAE_HOME:-$XDG_CONFIG/Trae}"
    ;;
esac

# ------------------- 工具清单: "类型|源路径|仓库内目标" -------------------
# sqlite: python3 backup API 一致性快照(应用运行中也可安全备份)
# copy:   文件/目录复制(目录走 rsync, 自动排除垃圾)
# glob:   逐个匹配的 sqlite 做快照, 目标为 <目标目录>/<上级目录名>.sqlite
declare -A TOOL_LABEL TOOL_BASE TOOL_PROC

TOOL_LABEL[zcode]="ZCode"
TOOL_BASE[zcode]="$ZCODE_HOME"
TOOL_PROC[zcode]="zcode"
ITEMS_ZCODE=(
  "sqlite|$ZCODE_HOME/cli/db/db.sqlite|zcode/db/db.sqlite"
  "copy|$ZCODE_HOME/agents|zcode/agents"
  "copy|$ZCODE_HOME/cli/config.json|zcode/config.json"
  "copy|$ZCODE_HOME/cli/memories|zcode/memories"
  "copy|$ZCODE_HOME/cli/plugins/data|zcode/plugins-data"
)
if [ "$SYNC_ROLLOUT" = 1 ]; then
  ITEMS_ZCODE+=( "copy|$ZCODE_HOME/cli/rollout|zcode/rollout" )
fi
if [ "$SYNC_ARTIFACTS" = 1 ]; then
  ITEMS_ZCODE+=( "copy|$ZCODE_HOME/cli/artifacts|zcode/artifacts" )
fi
if [ "$SYNC_WORKSPACES" = 1 ]; then
  ITEMS_ZCODE+=( "copy|$ZCODE_HOME/workspace|zcode/workspace" )
fi

TOOL_LABEL[claude]="Claude Code"
TOOL_BASE[claude]="$CLAUDE_HOME"
TOOL_PROC[claude]="claude"
ITEMS_CLAUDE=(
  "copy|$CLAUDE_HOME/projects|claude/projects"
  "copy|$CLAUDE_HOME/history.jsonl|claude/history.jsonl"
  "copy|$CLAUDE_HOME/todos|claude/todos"
  "copy|$CLAUDE_HOME/settings.json|claude/settings.json"
  "copy|$CLAUDE_HOME/CLAUDE.md|claude/CLAUDE.md"
)

TOOL_LABEL[cursor]="Cursor"
TOOL_BASE[cursor]="$CURSOR_HOME"
TOOL_PROC[cursor]="cursor"
ITEMS_CURSOR=(
  "sqlite|$CURSOR_HOME/User/globalStorage/state.vscdb|cursor/globalStorage/state.vscdb"
  "glob|$CURSOR_HOME/User/workspaceStorage/*/state.vscdb|cursor/workspaceStorage"
  "copy|$CURSOR_HOME/User/settings.json|cursor/settings.json"
  "copy|$CURSOR_HOME/User/keybindings.json|cursor/keybindings.json"
)

TOOL_LABEL[workbuddy]="WorkBuddy"
TOOL_BASE[workbuddy]="$WORKBUDDY_HOME"
TOOL_PROC[workbuddy]="workbuddy"
TOOL_LABEL[doubao]="豆包 (Doubao 桌面版)"
TOOL_BASE[doubao]="$DOUBAO_HOME"
TOOL_PROC[doubao]="doubao"
TOOL_LABEL[trae]="Trae (豆包系 IDE)"
TOOL_BASE[trae]="$TRAE_HOME"
TOOL_PROC[trae]="trae"
ITEMS_DOUBAO=(
  "copy|$DOUBAO_HOME|doubao"
)
ITEMS_WORKBUDDY=(
  "copy|$WORKBUDDY_HOME|workbuddy"
)
ITEMS_TRAE=(
  "sqlite|$TRAE_HOME/User/globalStorage/state.vscdb|trae/globalStorage/state.vscdb"
  "glob|$TRAE_HOME/User/workspaceStorage/*/state.vscdb|trae/workspaceStorage"
  "copy|$TRAE_HOME/User/settings.json|trae/settings.json"
)

# DeepSeek Harness: 会话为 zstd 压缩的 jsonl (sessions/), 其余为小状态文件
# 排除可重建的 profiles/(457M)/electron/(313M)/skins 等
TOOL_LABEL[dsh]="DeepSeek Harness (dsh)"
TOOL_BASE[dsh]="$DSH_HOME"
TOOL_PROC[dsh]="dsh"
ITEMS_DSH=(
  "copy|$DSH_HOME/sessions|dsh/sessions"
  "copy|$DSH_HOME/storages|dsh/storages"
  "copy|$DSH_HOME/dsh-session-archive|dsh/session-archive"
  "copy|$DSH_HOME/dsh-agents|dsh/agents"
  "copy|$DSH_HOME/dsh-usage|dsh/usage"
  "copy|$DSH_HOME/task-board|dsh/task-board"
)
if [ "$SYNC_DSH_CREDENTIALS" = 1 ]; then
  ITEMS_DSH+=(
    "copy|$DSH_HOME/.credentials.yaml|dsh/.credentials.yaml"
    "copy|$DSH_HOME/openai-gateway/api-key|dsh/openai-gateway-api-key"
  )
fi

# ------------------- 通用排除项 -------------------
EXCLUDES=(
  --exclude '.git/'        # 用户项目自己的 git 历史, 请用各自 remote 同步
  --exclude 'node_modules/'
  --exclude '__pycache__/'
  --exclude '.venv/' --exclude 'venv/'
  --exclude 'dist/' --exclude 'build/' --exclude 'target/'
  --exclude '.next/' --exclude '.cache/'
  --exclude '*.log'
  --exclude 'session.lock'   # dsh 会话锁文件, 恢复后不该保留
  --exclude '.DS_Store'
  --exclude 'Cache/'
  --exclude 'Code Cache/'
  --exclude 'GPUCache/'
  --exclude 'DawnCache/'
  --exclude 'DawnWebGPUCache/'
)

c_info() { printf '\033[36m%s\033[0m\n' "$*"; }
c_ok()   { printf '\033[32m%s\033[0m\n' "$*"; }
c_warn() { printf '\033[33m%s\033[0m\n' "$*"; }
c_err()  { printf '\033[31m%s\033[0m\n' "$*" >&2; }

# 某工具是否启用: 开关为 1 且基础目录存在
tool_active() {
  local t="$1"
  eval "local on=\$TOOL_${t^^}"
  [ "$on" = "1" ] && [ -d "${TOOL_BASE[$t]}" ]
}

# ------------------- 快照/复制 -------------------
snapshot_sqlite() {  # src dst
  local src="$1" dst="$2"
  [ -f "$src" ] || return 0
  mkdir -p "$(dirname "$dst")"
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$src" "$dst" <<'PY'
import sqlite3, sys, os
src, dst = sys.argv[1], sys.argv[2]
con = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
b = sqlite3.connect(dst)
con.backup(b)          # 在线备份 API: 即使源库处于 WAL/运行中也得到一致快照
b.close(); con.close()
PY
  else
    c_warn "python3 不可用, 回退为直接拷贝 (建议退出应用后执行)"
    cp -f "$src" "$dst"
    local ext
    for ext in wal shm; do
      [ -f "$src-$ext" ] && cp -f "$src-$ext" "$dst-$ext"
    done
  fi
}

sync_item() {  # spec -> 把一个条目收集进仓库
  local spec="$1"
  local type="${spec%%|*}"; spec="${spec#*|}"
  local src="${spec%%|*}";  local dst="${spec#*|}"
  case "$type" in
    sqlite)
      snapshot_sqlite "$src" "$REPO_DIR/$dst" && c_ok "  ✓ [sqlite] $dst"
      ;;
    glob)
      local f base
      for f in $src; do
        [ -f "$f" ] || continue
        base="$(basename "$(dirname "$f")")"
        snapshot_sqlite "$f" "$REPO_DIR/$dst/$base.sqlite" && c_ok "  ✓ [sqlite] $dst/$base.sqlite"
      done
      ;;
    copy)
      if [ -d "$src" ]; then
        mkdir -p "$REPO_DIR/$dst"
        local rc=0
        if command -v rsync >/dev/null 2>&1; then
          rsync -a "${EXCLUDES[@]}" "$src/" "$REPO_DIR/$dst/" || rc=$?
        else
          # 无 rsync 的环境（如 Windows Git Bash）：退化为整目录复制
          cp -r "$src/." "$REPO_DIR/$dst/" 2>/dev/null || rc=$?
        fi
        case "$rc" in
          0)     c_ok "  ✓ [dir] $dst/" ;;
          23|24) c_warn "  ✓ [dir] $dst/ (个别文件无权限或已消失, 已跳过)" ;;
          *)     c_err "  ✗ [dir] $dst/ rsync 失败 (rc=$rc)"; return 1 ;;
        esac
      elif [ -f "$src" ]; then
        mkdir -p "$(dirname "$REPO_DIR/$dst")"
        cp -f "$src" "$REPO_DIR/$dst" && c_ok "  ✓ [file] $dst"
      fi
      ;;
  esac
}

# ------------------- 恢复 -------------------
restore_item() {  # spec [--mirror]
  local spec="$1" mirror="${2:-}"
  local type="${spec%%|*}"; spec="${spec#*|}"
  local src="${spec%%|*}";  local dst="${spec#*|}"
  local repo_path="$REPO_DIR/$dst"
  [ -e "$repo_path" ] || return 0
  mkdir -p "$(dirname "$src")"
  case "$type" in
    sqlite)
      if [ ! -f "$repo_path" ]; then return 0; fi
      local bak="/tmp/ai-sync-restore-backup-$(date +%Y%m%d-%H%M%S)"
      mkdir -p "$bak"
      [ -f "$src" ] && cp -f "$src" "$bak/$(basename "$src")"   # 恢复前留保险副本
      rm -f "$src-wal" "$src-shm"                                # 残留 WAL 必须清掉, 否则可能损坏
      cp -f "$repo_path" "$src"
      c_ok "  ✓ [sqlite] $dst -> $src (原文件备份在 $bak)"
      ;;
    copy)
      if [ -d "$repo_path" ]; then
        local del=() rc=0
        [ "$mirror" = "--mirror" ] && del=(--delete)
        if command -v rsync >/dev/null 2>&1; then
          rsync -a "${del[@]}" "${EXCLUDES[@]}" "$repo_path/" "$src/" || rc=$?
        else
          # 无 rsync 的环境：整目录复制；--mirror 时先清空目标
          if [ "$mirror" = "--mirror" ] && [ -n "$src" ]; then
            rm -rf "${src:?}"/* 2>/dev/null || true
          fi
          mkdir -p "$src"
          cp -r "$repo_path/." "$src/" 2>/dev/null || rc=$?
        fi
        case "$rc" in
          0)     c_ok "  ✓ [dir] $dst -> $src/" ;;
          23|24) c_warn "  ✓ [dir] $dst -> $src/ (个别文件无权限, 已跳过)" ;;
          *)     c_err "  ✗ [dir] $dst -> $src/ rsync 失败 (rc=$rc)"; return 1 ;;
        esac
      elif [ -f "$repo_path" ]; then
        cp -f "$repo_path" "$src" && c_ok "  ✓ [file] $dst -> $src"
      fi
      ;;
    glob)
      return 0 ;;  # glob 由各 sqlite 条目在 collect 时展开, 恢复走 repo 内已有文件
  esac
}

restore_glob_children() {  # 工具名 [--mirror]: 把 repo 中 glob 产物恢复回各 workspaceStorage/<id>.sqlite
  local tool="$1" mirror="${2:-}"
  local spec t d repo_path f wsid target_root
  local specs=()
  eval "specs=(\"\${ITEMS_${tool^^}[@]}\")"
  for spec in "${specs[@]}"; do
    t="${spec%%|*}"
    [ "$t" = "glob" ] || continue
    spec="${spec#*|}"; local src="${spec%%|*}"; d="${spec#*|}"
    repo_path="$REPO_DIR/$d"
    [ -d "$repo_path" ] || continue
    target_root="${src%\*/state.vscdb}"
    [ "$mirror" = "--mirror" ] || true
    for f in "$repo_path"/*.sqlite; do
      [ -f "$f" ] || continue
      wsid="$(basename "$f" .sqlite)"
      mkdir -p "$target_root/$wsid"
      local tgt="$target_root/$wsid/state.vscdb"
      rm -f "$tgt-wal" "$tgt-shm"
      cp -f "$f" "$tgt"
      c_ok "  ✓ [sqlite] $d/$wsid.sqlite -> $tgt"
    done
  done
}

# ------------------- git 基础 -------------------
ensure_git_identity() {
  git -C "$REPO_DIR" config user.email >/dev/null 2>&1 || \
    git -C "$REPO_DIR" config user.email "ai-sync@$(hostname)"
  git -C "$REPO_DIR" config user.name >/dev/null 2>&1 || \
    git -C "$REPO_DIR" config user.name "ai-sync"
}

ensure_repo() {
  if [ ! -d "$REPO_DIR/.git" ]; then
    # 统一仓库模式：REPO_DIR 位于另一个 git 仓库内（例如工作台的数据仓库）。
    # 此时只负责收集数据，提交/推送由外层仓库统一完成。
    if [ -n "$(git -C "$REPO_DIR" rev-parse --show-toplevel 2>/dev/null)" ]; then
      UNIFIED_REPO=1
      mkdir -p "$REPO_DIR/zcode"
      printf '*.sqlite-wal\n*.sqlite-shm\n' > "$REPO_DIR/.gitignore"
      return 0
    fi
    c_err "仓库未初始化, 请先执行: $0 init"
    exit 1
  fi
  ensure_git_identity
  mkdir -p "$REPO_DIR/zcode"
  printf '*.sqlite-wal\n*.sqlite-shm\n' > "$REPO_DIR/.gitignore"
}

# ------------------- 子命令 -------------------
do_collect() {
  local tool spec
  local specs=()
  for tool in zcode claude cursor trae doubao workbuddy dsh; do
    tool_active "$tool" || continue
    c_info "▸ 收集 ${TOOL_LABEL[$tool]} ..."
    eval "specs=(\"\${ITEMS_${tool^^}[@]}\")"
    for spec in "${specs[@]}"; do
      sync_item "$spec"
    done
  done
}

cmd_push() {
  ensure_repo
  do_collect
  if [ "${UNIFIED_REPO:-0}" = "1" ]; then
    c_ok "已收集到统一仓库 $REPO_DIR（由外层仓库统一提交推送）。"
    return 0
  fi
  git -C "$REPO_DIR" add -A
  if git -C "$REPO_DIR" diff --cached --quiet; then
    c_info "没有变化, 无需提交。"
  else
    local n; n=$(git -C "$REPO_DIR" diff --cached --numstat | wc -l)
    git -C "$REPO_DIR" commit -q -m "sync @ $(hostname) $(date '+%F %T') — ${n} 个文件变更"
    c_ok "已提交 $n 个文件的变更。"
  fi
  local branch; branch=$(git -C "$REPO_DIR" symbolic-ref --short HEAD)
  if git -C "$REPO_DIR" remote get-url origin >/dev/null 2>&1; then
    git -C "$REPO_DIR" fetch origin --quiet 2>/dev/null || true
    if git -C "$REPO_DIR" rev-parse --verify -q "origin/$branch" >/dev/null; then
      git -C "$REPO_DIR" pull --no-rebase -X ours --no-edit --quiet || true  # 二进制冲突以本机为准
    fi
    git -C "$REPO_DIR" push -u origin "$branch" --quiet && c_ok "已推送到远程仓库。"
  else
    c_warn "尚未绑定远程仓库(仅保存在本机)。绑定方法:"
    c_info "  gh repo create ai-session-sync --private --source $REPO_DIR --push"
    c_info "  或: git -C $REPO_DIR remote add origin <你的私有仓库URL>"
  fi
}

cmd_pull() {
  local force=0 mirror=""
  [ "${1:-}" = "--force" ] && { force=1; shift; }
  [ "${1:-}" = "--mirror" ] && { mirror="--mirror"; shift; }
  ensure_repo
  local branch; branch=$(git -C "$REPO_DIR" symbolic-ref --short HEAD)
  if git -C "$REPO_DIR" rev-parse --verify -q "@{u}" >/dev/null; then
    git -C "$REPO_DIR" pull --no-rebase --no-edit --quiet && c_ok "已拉取远程更新。"
  else
    c_warn "没有配置上游, 跳过 git pull, 直接按本地仓库内容恢复。"
  fi
  local tool spec running
  local specs=()
  for tool in zcode claude cursor trae doubao workbuddy dsh; do
    tool_active "$tool" || continue
    running=$(pgrep -fi "${TOOL_PROC[$tool]}" >/dev/null 2>&1 && echo yes || echo no)
    c_info "▸ 恢复 ${TOOL_LABEL[$tool]} ...(应用运行中: $running)"
    eval "specs=(\"\${ITEMS_${tool^^}[@]}\")"
    for spec in "${specs[@]}"; do
      local type="${spec%%|*}"
      if [ "$type" = "sqlite" ] && [ "$running" = "yes" ] && [ "$force" != "1" ]; then
        c_warn "  ⚠ 跳过 sqlite 恢复(${TOOL_LABEL[$tool]} 正在运行)。请先退出应用, 或加 --force"
        continue
      fi
      if [ "$type" = "glob" ]; then
        restore_glob_children "$tool" "$mirror"
      else
        restore_item "$spec" "$mirror"
      fi
    done
  done
  c_ok "恢复完成。如涉及 ZCode 数据库, 重启 ZCode 后生效。"
}

cmd_status() {
  local tool spec active=()
  c_info "== 工具检测 =="
  for tool in zcode claude cursor trae doubao workbuddy dsh; do
    if tool_active "$tool"; then
      c_ok "  [√] ${TOOL_LABEL[$tool]}"
      active+=("$tool")
    else
      c_info "  [ ] ${TOOL_LABEL[$tool]} (未安装或已禁用)"
    fi
  done
  if [ ! -d "$REPO_DIR/.git" ]; then
    c_warn "仓库未初始化: $0 init"
    return 0
  fi
  c_info "== 仓库状态 ($REPO_DIR) =="
  local branch; branch=$(git -C "$REPO_DIR" symbolic-ref --short HEAD)
  echo "  分支: $branch"
  echo "  最近提交: $(git -C "$REPO_DIR" log -1 --format='%h %ad %s' --date=format:'%F %T' 2>/dev/null || echo '暂无提交')"
  local unpushed
  unpushed=$(git -C "$REPO_DIR" log "@{u}.." --oneline 2>/dev/null | wc -l)
  echo "  待推送提交: $unpushed"
  local dirty; dirty=$(git -C "$REPO_DIR" status --porcelain | wc -l)
  [ "$dirty" -gt 0 ] && c_warn "  有 $dirty 个未提交变更(执行 push 同步)" || c_ok "  工作区干净"
  git -C "$REPO_DIR" remote get-url origin >/dev/null 2>&1 \
    && echo "  远程: $(git -C "$REPO_DIR" remote get-url origin)" \
    || c_warn "  远程: 未绑定(数据仅在本机!)"
}

cmd_squash() {
  ensure_repo
  local branch; branch=$(git -C "$REPO_DIR" symbolic-ref --short HEAD)
  c_warn "将把历史压缩为单个提交(远程需 force push)。确认仓库没有他人协作。"
  git -C "$REPO_DIR" checkout -q --orphan "_fresh_$$_"
  git -C "$REPO_DIR" add -A
  git -C "$REPO_DIR" commit -q -m "squashed history @ $(date '+%F %T')"
  git -C "$REPO_DIR" branch -D "$branch" -q 2>/dev/null || git -C "$REPO_DIR" branch -D "$branch"
  git -C "$REPO_DIR" branch -m "$branch"
  c_ok "历史已压缩为 1 个提交。如绑定了远程, 执行: git -C $REPO_DIR push -f origin $branch"
}

cmd_init() {
  local remote=""
  [ "${1:-}" = "--remote" ] && remote="${2:-}"
  mkdir -p "$REPO_DIR"
  if [ ! -d "$REPO_DIR/.git" ]; then
    git -C "$REPO_DIR" init -q
    git -C "$REPO_DIR" symbolic-ref HEAD refs/heads/main
  fi
  ensure_git_identity
  printf '*.sqlite-wal\n*.sqlite-shm\n' > "$REPO_DIR/.gitignore"
  if [ ! -f "$CONF" ]; then
    cat > "$CONF" <<EOF
# ai-session-sync 配置(取消注释/改值后生效)
SYNC_WORKSPACES=1
SYNC_ROLLOUT=1
SYNC_ARTIFACTS=0
SYNC_DSH_CREDENTIALS=0
# TOOL_ZCODE=1
# TOOL_CLAUDE=1
# TOOL_CURSOR=1
# TOOL_TRAE=1
# TOOL_DSH=1
EOF
  fi
  if [ -n "$remote" ]; then
    git -C "$REPO_DIR" remote add origin "$remote" 2>/dev/null \
      || git -C "$REPO_DIR" remote set-url origin "$remote"
    c_ok "远程仓库已绑定: $remote"
  fi
  c_ok "初始化完成: $REPO_DIR"
  c_info "下一步: $0 push   (首次建议先在 GitHub/Gitee 建一个【私有】仓库并绑定)"
}

# ------------------- 自动化: crontab 管理 -------------------
CRON_MARKER="# ai-session-sync-cron (由 sync.sh cron install 管理, 勿手改)"

cmd_cron() {
  local action="${1:-status}"
  case "$action" in
    install)
      mkdir -p "$HOME/.local/state"
      (
        crontab -l 2>/dev/null \
          | grep -v "ai-session-sync/sync.sh push" \
          | grep -vF "ai-session-sync-cron" || true
        echo "$CRON_MARKER"
        echo "0 * * * * $REPO_DIR/sync.sh push >> $HOME/.local/state/ai-session-sync.log 2>&1"
      ) | crontab -
      c_ok "已安装每小时自动 push (查看: crontab -l, 日志: ~/.local/state/ai-session-sync.log)"
      if pgrep -x cron >/dev/null 2>&1 || pgrep -x crond >/dev/null 2>&1 \
         || systemctl is-active cron >/dev/null 2>&1; then
        c_ok "cron 服务运行中"
      else
        c_warn "未检测到运行中的 cron 服务, 自动化可能不生效 (systemctl enable --now cron)"
      fi
      ;;
    remove)
      (
        crontab -l 2>/dev/null \
          | grep -v "ai-session-sync/sync.sh push" \
          | grep -vF "ai-session-sync-cron" || true
      ) | crontab - || true
      c_ok "已移除自动 push 任务"
      ;;
    status)
      if crontab -l 2>/dev/null | grep -q "ai-session-sync/sync.sh push"; then
        c_ok "已安装: $(crontab -l 2>/dev/null | grep 'ai-session-sync/sync.sh push')"
      else
        c_info "未安装 (执行: $0 cron install)"
      fi
      ;;
    *)
      c_info "用法: $0 cron install|remove|status"
      ;;
  esac
}

# ------------------- 入口 -------------------
case "${1:-}" in
  init)   shift; cmd_init "$@" ;;
  push)   cmd_push ;;
  pull)   shift; cmd_pull "$@" ;;
  status) cmd_status ;;
  squash) cmd_squash ;;
  cron)   shift; cmd_cron "$@" ;;
  *) grep '^#   sync.sh' "$0" | sed 's/^#   //' ;;
esac
