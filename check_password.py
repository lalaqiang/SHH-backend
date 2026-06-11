#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
查询000064用户的当前密码格式
"""

import pyodbc

print("=" * 70)
print("查询000064用户密码")
print("=" * 70)
print()

try:
    conn = pyodbc.connect(
        'DRIVER={SQL Server};'
        'SERVER=shenhuahui.f3322.org,1433;'
        'DATABASE=TestERP;'
        'UID=sa;'
        'PWD=sa123456'
    )
    cursor = conn.cursor()
    cursor.execute("SELECT EmpNo, EmpName, PassWordStr FROM tBas_Emp WHERE EmpNo = '000064'")
    row = cursor.fetchone()
    
    if row:
        print(f"EmpNo: {row[0]}")
        print(f"EmpName: {row[1]}")
        print(f"PassWordStr: {row[2]}")
        pwd_len = len(row[2]) if row[2] else 0
        print(f"长度: {pwd_len}")
        
        if row[2] and row[2].startswith("SHA256:"):
            print(f"格式: SHA256加密")
            print()
            print("⚠ 警告: 密码已被新ERP升级为SHA256格式")
            print("老ERP无法识别此格式，需要恢复为XOR加密")
        elif pwd_len == 16 and all(c in '0123456789ABCDEFabcdef' for c in (row[2] or '')):
            print(f"格式: XOR加密（老ERP格式）")
        elif pwd_len > 0:
            print(f"格式: 明文密码")
        else:
            print(f"格式: 空密码")
    else:
        print("未找到该用户")
    
    conn.close()
    print()
    print("=" * 70)
    
except Exception as e:
    print(f"连接失败: {e}")
    print("=" * 70)
