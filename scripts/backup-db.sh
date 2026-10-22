#!/usr/bin/env bash
# KaedeNiwa SQLite 在线备份 / 恢复
#
# 备份:
#   scripts/backup-db.sh backup /var/lib/kaede/kaede.db [outdir]
#
# 恢复（必须先停止服务；需要 --force 才会真正覆盖）:
#   scripts/backup-db.sh restore /var/lib/kaede/kaede.db /path/to/backup.db --force
set -euo pipefail

usage() {
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
}

command -v sqlite3 >/dev/null 2>&1 || {
    echo "错误: 未找到 sqlite3（apt-get install -y sqlite3）" >&2
    exit 1
}

integrity() {
    local path=$1
    local result
    result=$(sqlite3 "$path" 'PRAGMA integrity_check;')
    if [[ "$result" != "ok" ]]; then
        echo "integrity_check 失败: $path" >&2
        echo "$result" >&2
        exit 1
    fi
}

sql_quote() {
    printf "%s" "${1//\'/\'\'}"
}

mode=${1:-}
case "$mode" in
    backup)
        db=${2:-}
        outdir=${3:-}
        [[ -n "$db" && -f "$db" ]] || { echo "数据库不存在: ${db:-<empty>}" >&2; exit 1; }
        if [[ -z "$outdir" ]]; then
            outdir="$(dirname "$db")/backups"
        fi
        mkdir -p "$outdir"
        dest="$outdir/kaede-$(date +%Y%m%d-%H%M%S).db"

        echo "备份 $db -> $dest"
        sqlite3 "$db" ".backup '$(sql_quote "$dest")'"
        integrity "$dest"
        ( cd "$outdir" && sha256sum "$(basename "$dest")" > "$(basename "$dest").sha256" )

        echo "完成: $dest"
        cat "$dest.sha256"
        ;;
    restore)
        db=${2:-}
        backup=${3:-}
        force=${4:-}
        [[ -n "$db" && -n "$backup" ]] || usage
        [[ -f "$backup" ]] || { echo "备份文件不存在: $backup" >&2; exit 1; }
        integrity "$backup"

        if [[ "$force" != "--force" ]]; then
            echo "未指定 --force，仅做校验，不执行恢复。"
            echo "确认服务已停止后，追加 --force 重新执行。"
            exit 2
        fi

        rm -f "$db-wal" "$db-shm"
        echo "恢复 $backup -> $db"
        sqlite3 "$db" ".restore '$(sql_quote "$backup")'"
        rm -f "$db-wal" "$db-shm"
        integrity "$db"
        echo "恢复完成，integrity_check = ok"
        ;;
    *)
        usage
        ;;
esac
