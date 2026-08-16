#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""为非 admin 员工分配系统管理员角色并设置密码，用于测试菜单权限修复效果"""

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

TEST_EMP_NO = "000000"  # 周陆
TEST_PASSWORD = "123456"  # 与 admin 相同，便于测试

try:
    conn = pyodbc.connect(conn_str, autocommit=False)
    cursor = conn.cursor()

    # 1. 获取 admin 的密码 hash（用于复制给测试用户，确保密码相同）
    cursor.execute("SELECT PassWordStr FROM tBas_Emp WHERE EmpNo = 'admin'")
    admin_row = cursor.fetchone()
    if not admin_row or not admin_row.PassWordStr:
        print("错误: 无法获取 admin 的密码 hash")
        raise SystemExit(1)
    admin_pwd_hash = admin_row.PassWordStr
    print(f"获取 admin 密码 hash: {admin_pwd_hash[:30]}...（长度 {len(admin_pwd_hash)}）")

    # 2. 获取测试员工的 EmpID
    cursor.execute("SELECT EmpID, EmpName FROM tBas_Emp WHERE EmpNo = ?", (TEST_EMP_NO,))
    emp_row = cursor.fetchone()
    if not emp_row:
        print(f"错误: 未找到工号为 {TEST_EMP_NO} 的员工")
        raise SystemExit(1)
    test_emp_id = str(emp_row.EmpID)
    test_emp_name = emp_row.EmpName
    print(f"测试员工: {TEST_EMP_NO} | {test_emp_name} | EmpID={test_emp_id}")

    # 3. 给测试员工设置与 admin 相同的密码
    cursor.execute("UPDATE tBas_Emp SET PassWordStr = ? WHERE EmpNo = ?", (admin_pwd_hash, TEST_EMP_NO))
    print(f"已为 {TEST_EMP_NO} 设置密码（与 admin 相同）")

    # 4. 查找"系统管理员"角色的 RuleID
    cursor.execute("SELECT RuleID FROM tSys_Rule WHERE RuleName = '系统管理员'")
    role_row = cursor.fetchone()
    if not role_row:
        print("错误: 未找到系统管理员角色")
        raise SystemExit(1)
    admin_role_id = str(role_row.RuleID)
    print(f"系统管理员角色 RuleID: {admin_role_id}")

    # 5. 检查测试员工是否已分配角色
    cursor.execute("SELECT UserRuleID FROM tSys_UserRule WHERE EmpID = ?", (test_emp_id,))
    existing = cursor.fetchone()
    if existing:
        # 已有分配，先删除
        cursor.execute("DELETE FROM tSys_UserRule WHERE EmpID = ?", (test_emp_id,))
        print(f"已删除 {TEST_EMP_NO} 的旧角色分配")

    # 6. 分配系统管理员角色给测试员工
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    cursor.execute("""
        INSERT INTO tSys_UserRule (UserRuleID, EmpID, RuleID, LUTime)
        VALUES (NEWID(), ?, ?, ?)
    """, (test_emp_id, admin_role_id, now))
    print(f"已为 {TEST_EMP_NO} 分配 [系统管理员] 角色")

    conn.commit()
    print(f"\n测试用户准备完成：")
    print(f"  工号: {TEST_EMP_NO}")
    print(f"  姓名: {test_emp_name}")
    print(f"  密码: {TEST_PASSWORD}")
    print(f"  角色: 系统管理员（97 个菜单，全权限）")
    print(f"\n现在可以用 {TEST_EMP_NO}/{TEST_PASSWORD} 登录系统测试菜单显示效果。")

except Exception as e:
    print(f"错误: {e}")
    if 'conn' in locals():
        conn.rollback()
finally:
    if 'conn' in locals():
        conn.close()
