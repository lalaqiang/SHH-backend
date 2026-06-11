#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
反推老ERP的XOR加密密钥
已知：密码明文 "123456"，加密后 "07699FF9D41F0BAD"
"""

def reverse_xor_key(plaintext: str, ciphertext: str) -> list:
    """根据明文和密文反推XOR密钥"""
    plain_bytes = plaintext.encode('utf-8').ljust(8, b'\x00')
    cipher_bytes = bytes(int(ciphertext[i:i+2], 16) for i in range(0, 16, 2))
    
    key = []
    for i in range(8):
        key_byte = plain_bytes[i] ^ cipher_bytes[i]
        key.append(key_byte)
    
    return key

def encrypt_with_new_key(password: str, key: list) -> str:
    """使用新密钥加密"""
    pwd_bytes = password.encode('utf-8').ljust(8, b'\x00')
    encrypted = []
    for i in range(8):
        encrypted.append(pwd_bytes[i] ^ key[i])
    return ''.join(f'{b:02X}' for b in encrypted)

def decrypt_with_new_key(ciphertext: str, key: list) -> str:
    """使用新密钥解密"""
    cipher_bytes = bytes(int(ciphertext[i:i+2], 16) for i in range(0, 16, 2))
    decrypted = []
    for i in range(8):
        decrypted.append(cipher_bytes[i] ^ key[i])
    result = bytes(decrypted).split(b'\x00')[0]
    return result.decode('utf-8')

def main():
    print("=" * 60)
    print("反推老ERP XOR加密密钥")
    print("=" * 60)
    print()
    
    # 已知信息
    plaintext = "123456"
    ciphertext = "07699FF9D41F0BAD"
    
    print(f"已知明文: {plaintext}")
    print(f"已知密文: {ciphertext}")
    print()
    
    # 反推密钥
    key = reverse_xor_key(plaintext, ciphertext)
    key_hex = [f'0x{b:02X}' for b in key]
    
    print("=" * 60)
    print("反推结果：")
    print("=" * 60)
    print()
    print(f"老ERP XOR密钥: {', '.join(key_hex)}")
    print()
    print("Rust代码格式:")
    print(f"const LEGACY_XOR_KEY: [u8; 8] = [{', '.join(key_hex)}];")
    print()
    print("Python代码格式:")
    print(f"LEGACY_XOR_KEY = {key}")
    print()
    
    # 验证
    print("=" * 60)
    print("验证：")
    print("=" * 60)
    encrypted = encrypt_with_new_key(plaintext, key)
    decrypted = decrypt_with_new_key(ciphertext, key)
    
    print(f"明文 {plaintext} 加密后: {encrypted}")
    print(f"与数据库值对比: {ciphertext}")
    print(f"匹配: {'✓ 是' if encrypted == ciphertext else '✗ 否'}")
    print()
    print(f"密文 {ciphertext} 解密后: {decrypted}")
    print(f"与原明文对比: {plaintext}")
    print(f"匹配: {'✓ 是' if decrypted == plaintext else '✗ 否'}")
    print()
    
    # 测试其他常见密码
    print("=" * 60)
    print("常见密码使用新密钥的加密值：")
    print("=" * 60)
    test_passwords = ["admin", "888888", "000064", "password", "123123"]
    for pwd in test_passwords:
        enc = encrypt_with_new_key(pwd, key)
        print(f"  {pwd:10s} -> {enc}")
    print()
    
    # 生成修改建议
    print("=" * 60)
    print("修改建议：")
    print("=" * 60)
    print()
    print("需要修改以下文件中的 LEGACY_XOR_KEY：")
    print("1. server-rust/src/handlers/auth.rs")
    print("2. server-rust/src/handlers/mobile.rs")
    print()
    print(f"将原来的:")
    print(f"const LEGACY_XOR_KEY: [u8; 8] = [0xFC, 0xAA, 0x62, 0xA0, 0x30, 0x9C, 0xF1, 0xD4];")
    print()
    print(f"修改为:")
    print(f"const LEGACY_XOR_KEY: [u8; 8] = [{', '.join(key_hex)}];")

if __name__ == "__main__":
    main()
