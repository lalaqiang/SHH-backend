#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
用老ERP的XOR密钥加密密码，生成正确的PassWordStr值
"""

# 老ERP的XOR密钥（从之前的分析中得到）
LEGACY_XOR_KEY = [0x36, 0x5B, 0xAC, 0xCD, 0xE1, 0x29, 0x0B, 0xAD]

def encrypt_password(plaintext: str) -> str:
    """XOR加密密码"""
    plain_bytes = plaintext.encode('utf-8')
    # 补齐到8字节
    plain_bytes = plain_bytes.ljust(8, b'\x00')
    
    encrypted = []
    for i in range(8):
        encrypted.append(plain_bytes[i] ^ LEGACY_XOR_KEY[i])
    
    # 转为16进制字符串（大写）
    return ''.join(f'{b:02X}' for b in encrypted)

# 计算常用密码的加密值
passwords = ['123456', '115580', 'admin', '1234']

print("=" * 70)
print("老ERP密码XOR加密结果")
print("=" * 70)
print()

for pwd in passwords:
    encrypted = encrypt_password(pwd)
    print(f"明文密码: {pwd}")
    print(f"加密结果: {encrypted}")
    print(f"长度: {len(encrypted)}")
    print()

print("=" * 70)
print("请在SQL Server中执行以下命令：")
print("=" * 70)
print()
print("-- 恢复admin密码为 115580")
print(f"UPDATE tBas_Emp SET PassWordStr = '{encrypt_password('115580')}' WHERE EmpNo = 'admin';")
print()
print("-- 恢复000064密码为 123456")
print(f"UPDATE tBas_Emp SET PassWordStr = '{encrypt_password('123456')}' WHERE EmpNo = '000064';")
print()
print("-- 验证修改结果")
print("SELECT EmpNo, EmpName, PassWordStr, LEN(PassWordStr) FROM tBas_Emp WHERE EmpNo IN ('admin', '000064');")
