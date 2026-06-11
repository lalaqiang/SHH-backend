#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
测试新的密码验证逻辑：O和0自动替换
"""

# 老ERP的XOR密钥
LEGACY_XOR_KEY = [0x36, 0x5B, 0xAC, 0xCD, 0xE1, 0x29, 0x0B, 0xAD]

def decrypt_legacy_password(stored: str) -> str:
    """解密XOR加密的密码"""
    try:
        bytes_list = [int(stored[i:i+2], 16) for i in range(0, 16, 2)]
        decrypted = []
        for i, b in enumerate(bytes_list):
            decrypted.append(b ^ LEGACY_XOR_KEY[i])
        result = bytes(decrypted).split(b'\x00')[0]
        return result.decode('utf-8')
    except:
        return None

def verify_password_new(password: str, stored: str) -> bool:
    """新ERP的密码验证逻辑（兼容O和0）"""
    # 方式1: SHA256
    if stored.startswith("SHA256:"):
        # 简化处理，不实现SHA256
        return False
    
    # 方式2: XOR加密（兼容O/o替换为0）
    normalized_stored = stored.replace('O', '0').replace('o', '0')
    
    if len(normalized_stored) == 16 and all(c in '0123456789ABCDEFabcdef' for c in normalized_stored):
        decrypted = decrypt_legacy_password(normalized_stored)
        if decrypted:
            print(f"  XOR解密（规范化后）: {normalized_stored} -> {decrypted}")
            return decrypted == password
    
    # 方式3: 空密码
    if not stored:
        return False
    
    # 方式4: 明文比较
    return password == stored

# 测试
print("=" * 60)
print("测试新密码验证逻辑（O/0自动替换）")
print("=" * 60)
print()

# 测试用例1: 包含字母O的密码
stored_pwd = "07699FF9D41FOBAD"  # 包含字母O
print(f"测试1: 密码包含字母O")
print(f"  数据库值: {stored_pwd}")
print(f"  尝试密码: 123456")
result = verify_password_new("123456", stored_pwd)
print(f"  验证结果: {'✓ 成功' if result else '✗ 失败'}")
print()

# 测试用例2: 正常的十六进制密码
stored_pwd2 = "07699FF9D41F0BAD"  # 正常的数字0
print(f"测试2: 正常的十六进制密码")
print(f"  数据库值: {stored_pwd2}")
print(f"  尝试密码: 123456")
result2 = verify_password_new("123456", stored_pwd2)
print(f"  验证结果: {'✓ 成功' if result2 else '✗ 失败'}")
print()

# 测试用例3: 明文密码
stored_pwd3 = "123456"
print(f"测试3: 明文密码")
print(f"  数据库值: {stored_pwd3}")
print(f"  尝试密码: 123456")
result3 = verify_password_new("123456", stored_pwd3)
print(f"  验证结果: {'✓ 成功' if result3 else '✗ 失败'}")
print()

print("=" * 60)
print("结论:")
print("=" * 60)
if result and result2:
    print("✓ 新逻辑可以同时处理包含O和正常0的密码")
    print("✓ 老ERP的数据可以正常迁移到新ERP")
else:
    print("✗ 逻辑有问题，需要修改")
