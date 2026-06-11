#!/usr/bin/env bash
# 库存预警定时检查 - Linux/macOS 脚本
# 用法：手动运行  /  配合 cron 调度（每天 8:00 跑一次）
#
# 添加到 crontab -e：
#   0 8 * * * /opt/erp/scripts/low_stock_alert.sh >> /var/log/erp_low_stock.log 2>&1

set -e

API_BASE="${API_BASE:-http://localhost:8080}"
TOKEN="${ERP_TOKEN:-}"
ALERT_THRESHOLD="${ALERT_THRESHOLD:-0}"

echo "[$(date '+%Y-%m-%d %H:%M:%S')] 正在检查库存预警..."

# 1) 调预警接口
ALERT=$(curl -sS -X POST "$API_BASE/api/inventory/low_stock_alert" \
    -H "Content-Type: application/json" \
    ${TOKEN:+-H "Authorization: Bearer $TOKEN"} \
    -d '{}' --max-time 30)

if [ $? -ne 0 ]; then
    echo "[ERROR] 调用预警接口失败" >&2
    exit 1
fi

# 简单解析（生产环境用 jq）
CRITICAL=$(echo "$ALERT" | grep -oP '"critical":\s*\K[0-9]+' | head -1)
WARNING=$(echo "$ALERT"  | grep -oP '"warning":\s*\K[0-9]+'  | head -1)
TOTAL=$(echo "$ALERT"    | grep -oP '"total":\s*\K[0-9]+'    | head -1)

REMINDER=$((TOTAL - CRITICAL - WARNING))
echo "  紧急: ${CRITICAL:-0}  警告: ${WARNING:-0}  提醒: ${REMINDER:-0}"

# 2) 紧急项超阈值 → 自动转补货
if [ "${CRITICAL:-0}" -gt "$ALERT_THRESHOLD" ]; then
    echo "[WARN] 紧急项 = ${CRITICAL} 超过阈值 = $ALERT_THRESHOLD，自动转补货申请..."
    CREATE=$(curl -sS -X POST "$API_BASE/api/inventory/replenish_from_alert" \
        -H "Content-Type: application/json" \
        ${TOKEN:+-H "Authorization: Bearer $TOKEN"} \
        -d '{}' --max-time 60)
    CREATED=$(echo "$CREATE" | grep -oP '"CreatedCount":\s*\K[0-9]+' | head -1)
    echo "  已生成补货申请 ${CREATED:-0} 张"
fi

echo "[DONE] 库存预警检查完成"
