#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""修复"系统管理员"角色的菜单权限记录

问题：
  - 当前"系统管理员"角色在 tSys_RuleMenu 中有 162 条记录（应为 97 条）
  - 存在冗余/不一致的菜单记录
  - 其他角色都是 0 条菜单（非 admin 用户登录后菜单为空）

修复方案：
  1. 删除"系统管理员"角色的所有 tSys_RuleMenu 记录
  2. 从 tSys_Menus 读取所有启用菜单（97 条）
  3. 为每个菜单插入一条全权限为 1 的记录（CanRead/CanCreate/.../CanExport = 1）
  4. 验证修复后记录数 = 菜单数

注意：
  - 此脚本只修复"系统管理员"角色，其他角色的菜单权限应由用户在前端"角色管理"页面手动配置
  - 分配了"系统管理员"角色的用户将能看到所有菜单
"""

import pyodbc
from datetime import datetime

conn_str = (
    "DRIVER={ODBC Driver 17 for SQL Server};"
    "SERVER=DESKTOP-QKTHTQP\\SQLEXPRESS;"
    "UID=sa;PWD=sa123456;"
    "DATABASE=TestERP;"
    "TrustServerCertificate=yes;"
    "Encrypt=no;"
)

ADMIN_ROLE_NAME = "系统管理员"

try:
    conn = pyodbc.connect(conn_str, autocommit=False)
    cursor = conn.cursor()
    print("数据库连接成功")

    # 1. 查找"系统管理员"角色的 RuleID
    cursor.execute("SELECT RuleID FROM tSys_Rule WHERE RuleName = ?", (ADMIN_ROLE_NAME,))
    row = cursor.fetchone()
    if not row:
        print(f"错误: 未找到角色 [{ADMIN_ROLE_NAME}]")
        raise SystemExit(1)
    admin_rule_id = str(row.RuleID)
    print(f"找到角色 [{ADMIN_ROLE_NAME}]: RuleID = {admin_rule_id}")

    # 2. 统计修复前的菜单记录数
    cursor.execute("SELECT COUNT(*) FROM tSys_RuleMenu WHERE RuleID = ?", (admin_rule_id,))
    before_count = cursor.fetchone()[0]
    print(f"修复前: tSys_RuleMenu 中该角色有 {before_count} 条记录")

    # 3. 读取 tSys_Menus 所有启用菜单
    cursor.execute("SELECT SYM_ID FROM tSys_Menus WHERE ISNULL(Used, 'Y') = 'Y'")
    menus = [str(row.SYM_ID) for row in cursor.fetchall()]
    print(f"tSys_Menus 启用菜单数: {len(menus)}")

    # 4. 删除旧记录
    cursor.execute("DELETE FROM tSys_RuleMenu WHERE RuleID = ?", (admin_rule_id,))
    deleted = cursor.rowcount
    print(f"已删除旧记录: {deleted} 条")

    # 5. 为每个菜单插入全权限记录
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    insert_sql = """INSERT INTO tSys_RuleMenu
        (RuleMenuID, RuleID, MenuID, CanRead, CanCreate, CanUpdate, CanDelete, CanAudit, CanPrint, CanExport, LUTime)
        VALUES (NEWID(), ?, ?, 1, 1, 1, 1, 1, 1, 1, ?)"""
    inserted = 0
    for menu_id in menus:
        cursor.execute(insert_sql, (admin_rule_id, menu_id, now))
        inserted += 1

    conn.commit()
    print(f"已插入新记录: {inserted} 条（每个菜单一条，全权限=1）")

    # 6. 验证
    cursor.execute("SELECT COUNT(*) FROM tSys_RuleMenu WHERE RuleID = ?", (admin_rule_id,))
    after_count = cursor.fetchone()[0]
    print(f"修复后: tSys_RuleMenu 中该角色有 {after_count} 条记录")
    if after_count == len(menus):
        print(f"✓ 验证通过: 记录数 {after_count} = 菜单数 {len(menus)}")
    else:
        print(f"✗ 验证失败: 记录数 {after_count} ≠ 菜单数 {len(menus)}")

    # 7. 抽样验证权限位
    cursor.execute("""
        SELECT TOP 3 m.SYM_CAPTION, rm.CanRead, rm.CanCreate, rm.CanUpdate, rm.CanDelete, rm.CanAudit, rm.CanPrint, rm.CanExport
        FROM tSys_RuleMenu rm
        INNER JOIN tSys_Menus m ON rm.MenuID = m.SYM_ID
        WHERE rm.RuleID = ?
        ORDER BY m.SYM_NO
    """, (admin_rule_id,))
    print("\n抽样验证（前 3 条）:")
    for r in cursor.fetchall():
        print(f"  {r.SYM_CAPTION}: Read={r.CanRead} Create={r.CanCreate} Update={r.CanUpdate} Delete={r.CanDelete} Audit={r.CanAudit} Print={r.CanPrint} Export={r.CanExport}")

    print(f"\n修复完成。现在分配了 [{ADMIN_ROLE_NAME}] 角色的用户将能看到所有 {len(menus)} 个菜单。")

except Exception as e:
    print(f"错误: {e}")
    if 'conn' in locals():
        conn.rollback()
finally:
    if 'conn' in locals():
        conn.close()
