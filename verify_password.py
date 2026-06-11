#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
ERP密码加密验证工具
支持验证XOR加密、SHA256加密和明文密码
"""

import hashlib

# XOR加密密钥（与Rust代码一致）
LEGACY_XOR_KEY = [0xFC, 0xAA, 0x62, 0xA0, 0x30, 0x9C, 0xF1, 0xD4]

def encrypt_xor(password: str) -> str:
    """XOR加密密码（老ERP使用）"""
    pwd_bytes = password.encode('utf-8')
    # 补齐到8字节
    pwd_bytes = pwd_bytes.ljust(8, b'\x00')
    
    encrypted = []
    for i, b in enumerate(pwd_bytes[:8]):
        encrypted.append(b ^ LEGACY_XOR_KEY[i])
    
    return ''.join(f'{b:02X}' for b in encrypted)

def decrypt_xor(stored: str) -> str:
    """解密XOR加密的密码"""
    if len(stored) != 16:
        return None
    
    try:
        bytes_list = [int(stored[i:i+2], 16) for i in range(0, 16, 2)]
        
        decrypted = []
        for i, b in enumerate(bytes_list):
            decrypted.append(b ^ LEGACY_XOR_KEY[i])
        
        # 去除末尾的0
        result = bytes(decrypted).split(b'\x00')[0]
        return result.decode('utf-8')
    except:
        return None

def hash_sha256(password: str) -> str:
    """SHA256加密密码（新ERP使用）"""
    salt = "erp_shenhuihui_2024"
    hash_value = hashlib.sha256(f"{password}{salt}".encode('utf-8')).hexdigest()
    return f"SHA256:{hash_value}"

def verify_password(password: str, stored: str) -> bool:
    """验证密码"""
    # 方式1: SHA256
    if stored.startswith("SHA256:"):
        return hash_sha256(password) == stored
    
    # 方式2: XOR加密
    if len(stored) == 16 and all(c in '0123456789ABCDEFabcdef' for c in stored):
        decrypted = decrypt_xor(stored)
        return decrypted == password
    
    # 方式3: 明文
    return stored == password

def main():
    print("=" * 60)
    print("ERP密码加密验证工具")
    print("=" * 60)
    print()
    
    # 测试密码
    test_password = "123456"
    print(f"测试密码: {test_password}")
    print()
    
    # XOR加密
    xor_encrypted = encrypt_xor(test_password)
    print(f"XOR加密结果: {xor_encrypted}")
    print(f"XOR解密验证: {decrypt_xor(xor_encrypted)}")
    print()
    
    # SHA256加密
    sha256_hash = hash_sha256(test_password)
    print(f"SHA256加密结果: {sha256_hash}")
    print()
    
    print("=" * 60)
    print("使用说明:")
    print("=" * 60)
    print("1. 在SQL Server中执行以下查询:")
    print("   SELECT PassWordStr FROM tBas_Emp WHERE EmpNo = '000064'")
    print()
    print("2. 将查询到的密码值粘贴到下方进行验证")
    print()
    
    # 常见密码的XOR加密值
    print("常见密码的XOR加密值:")
    test_cases = ["123456", "admin", "000064", "password", "888888"]
    for pwd in test_cases:
        print(f"  {pwd:10s} -> {encrypt_xor(pwd)}")
    print()
    
    # 交互式验证
    print("=" * 60)
    stored_password = input("请输入从数据库查询到的PassWordStr值（直接回车退出）: ").strip()
    
    if not stored_password:
        return
    
    print()
    print(f"数据库密码值: {stored_password}")
    print(f"密码长度: {len(stored_password)}")
    print()
    
    # 判断加密类型
    if stored_password.startswith("SHA256:"):
        print("✓ 加密类型: SHA256")
    elif len(stored_password) == 16 and all(c in '0123456789ABCDEFabcdef' for c in stored_password):
        print("✓ 加密类型: XOR加密")
        decrypted = decrypt_xor(stored_password)
        print(f"✓ 解密结果: {decrypted}")
    else:
        print("✓ 加密类型: 明文或其他")
    
    print()
    
    # 验证密码123456
    if verify_password("123456", stored_password):
        print("✓ 密码验证成功！帐号000064的密码确实是 123456")
    else:
        print("✗ 密码验证失败！123456 不是正确密码")
        print()
        print("尝试解密...")
        if len(stored_password) == 16 and all(c in '0123456789ABCDEFabcdef' for c in stored_password):
            decrypted = decrypt_xor(stored_password)
            if decrypted:
                print(f"解密后的密码是: {decrypted}")

if __name__ == "__main__":
    main()
