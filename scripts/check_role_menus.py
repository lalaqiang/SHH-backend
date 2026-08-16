#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""检查当前角色权限分配情况，诊断非 admin 用户菜单为空问题"""

import pyodbc

conn_str = (
    "DRIVER={ODBC Driver 17 for SQL Server};"
    "SERVER=DESKTOP-QKTHTQP\\SQLEXPRESS;"
    "UID=sa;PWD=sa123456;"
    "DATABASE=TestERP;"
    "TrustServerCertificate=yes;"
    "Encrypt=no;"
)

try:
    conn = pyodbc.connect(conn_str, autocommit=True)
    cursor = conn.cursor()
    print("=" * 70)
    print("1. 角色列表 (tSys_Rule)")
    print("=" * 70)
    cursor.execute("SELECT RuleID, RuleName, State FROM tSys_Rule ORDER BY RuleName")
    roles = cursor.fetchall()
    for r in roles:
        print(f"  {r.RuleID} | {r.RuleName} | State={r.State}")

    print()
    print("=" * 70)
    print("2. 每个角色的菜单数量 (tSys_RuleMenu)")
    print("=" * 70)
    cursor.execute("""
        SELECT r.RuleName, COUNT(rm.MenuID) AS MenuCount
        FROM tSys_Rule r
        LEFT JOIN tSys_RuleMenu rm ON r.RuleID = rm.RuleID
        GROUP BY r.RuleName, r.RuleID
        ORDER BY r.RuleName
    """)
    for r in cursor.fetchall():
        print(f"  {r.RuleName}: {r.MenuCount} 个菜单")

    print()
    print("=" * 70)
    print("3. 用户-角色分配 (tSys_UserRule)")
    print("=" * 70)
    cursor.execute("""
        SELECT e.EmpNo, e.EmpName, r.RuleName
        FROM tSys_UserRule ur
        INNER JOIN tBas_Emp e ON ur.EmpID = e.EmpID
        INNER JOIN tSys_Rule r ON ur.RuleID = r.RuleID
        ORDER BY e.EmpNo
    """)
    user_rules = cursor.fetchall()
    if not user_rules:
        print("  (空) - 没有任何用户分配角色")
    for r in user_rules:
        print(f"  {r.EmpNo} | {r.EmpName} | 角色: {r.RuleName}")

    print()
    print("=" * 70)
    print("4. tSys_Menus 总数")
    print("=" * 70)
    cursor.execute("SELECT COUNT(*) AS Total FROM tSys_Menus")
    print(f"  菜单总数: {cursor.fetchone().Total}")

    print()
    print("=" * 70)
    print("5. 非 admin 用户的 EmpID（用于测试 get_my_permissions）")
    print("=" * 70)
    cursor.execute("""
        SELECT TOP 5 EmpNo, EmpName, EmpID, State
        FROM tBas_Emp
        WHERE EmpNo <> 'admin' AND State <> 'D'
        ORDER BY EmpNo
    """)
    non_admin = cursor.fetchall()
    if not non_admin:
        print("  (空) - 没有非 admin 员工")
    for r in non_admin:
        print(f"  {r.EmpNo} | {r.EmpName} | EmpID={r.EmpID} | State={r.State}")

    print()
    print("=" * 70)
    print("6. admin 用户信息")
    print("=" * 70)
    cursor.execute("SELECT EmpNo, EmpName, EmpID, State FROM tBas_Emp WHERE EmpNo = 'admin'")
    admin = cursor.fetchone()
    if admin:
        print(f"  {admin.EmpNo} | {admin.EmpName} | EmpID={admin.EmpID} | State={admin.State}")
    else:
        print("  admin 用户不存在")

except Exception as e:
    print(f"错误: {e}")
finally:
    if 'conn' in locals():
        conn.close()
