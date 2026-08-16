#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""查看非 admin 用户账号情况，找到可用于测试登录的用户"""

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
    print("1. tSys_User 表结构")
    print("=" * 70)
    cursor.execute("""
        SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH
        FROM INFORMATION_SCHEMA.COLUMNS
        WHERE TABLE_NAME = 'tSys_User'
        ORDER BY ORDINAL_POSITION
    """)
    for r in cursor.fetchall():
        print(f"  {r.COLUMN_NAME} ({r.DATA_TYPE}, max={r.CHARACTER_MAXIMUM_LENGTH})")

    print()
    print("=" * 70)
    print("2. tSys_User 所有账号")
    print("=" * 70)
    cursor.execute("""
        SELECT TOP 10 UserID, UserCode, UserName, EmpID, State
        FROM tSys_User
        ORDER BY UserCode
    """)
    users = cursor.fetchall()
    if not users:
        print("  (空) tSys_User 表无账号")
    for r in users:
        print(f"  {r.UserCode} | {r.UserName} | EmpID={r.EmpID} | State={r.State}")

    print()
    print("=" * 70)
    print("3. tBas_Emp 前 5 条（非 admin）")
    print("=" * 70)
    cursor.execute("""
        SELECT TOP 5 EmpID, EmpNo, EmpName, State
        FROM tBas_Emp
        WHERE EmpNo <> 'admin'
        ORDER BY EmpNo
    """)
    for r in cursor.fetchall():
        print(f"  EmpNo={r.EmpNo} | EmpName={r.EmpName} | EmpID={r.EmpID} | State={r.State}")

    print()
    print("=" * 70)
    print("4. tSys_User 表中是否有 UserCode 与 tBas_Emp.EmpNo 的关联")
    print("=" * 70)
    cursor.execute("""
        SELECT TOP 10 u.UserCode, u.UserName, e.EmpNo, e.EmpName
        FROM tSys_User u
        LEFT JOIN tBas_Emp e ON u.EmpID = e.EmpID
        ORDER BY u.UserCode
    """)
    for r in cursor.fetchall():
        print(f"  UserCode={r.UserCode} | UserName={r.UserName} | EmpNo={r.EmpNo} | EmpName={r.EmpName}")

except Exception as e:
    print(f"错误: {e}")
finally:
    if 'conn' in locals():
        conn.close()
