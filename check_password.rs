use std::collections::HashMap;

const LEGACY_XOR_KEY: [u8; 8] = [0xFC, 0xAA, 0x62, 0xA0, 0x30, 0x9C, 0xF1, 0xD4];

fn decrypt_legacy_password(stored: &str) -> Option<String> {
    let bytes: Vec<u8> = (0..stored.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&stored[i..i + 2], 16).ok())
        .collect();
    
    if bytes.len() != 8 {
        return None;
    }
    
    let decrypted: Vec<u8> = bytes.iter()
        .enumerate()
        .map(|(i, &b)| b ^ LEGACY_XOR_KEY[i])
        .collect();
    
    let trimmed = decrypted.iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect::<Vec<u8>>();
    
    String::from_utf8(trimmed).ok()
}

fn encrypt_legacy_password(password: &str) -> String {
    let mut bytes = password.as_bytes().to_vec();
    bytes.resize(8, 0);
    
    let encrypted: Vec<u8> = bytes.iter()
        .enumerate()
        .map(|(i, &b)| b ^ LEGACY_XOR_KEY[i])
        .collect();
    
    encrypted.iter()
        .map(|b| format!("{:02X}", b))
        .collect()
}

fn hash_sha256(password: &str) -> String {
    use sha2::{Sha256, Digest};
    let salt = "erp_shenhuihui_2024";
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", password, salt).as_bytes());
    let result = hasher.finalize();
    format!("SHA256:{}", hex::encode(result))
}

fn main() {
    println!("=== ERP密码加密分析工具 ===\n");
    
    // 测试1: 验证XOR加密
    let test_password = "123456";
    let encrypted = encrypt_legacy_password(test_password);
    println!("测试密码: {}", test_password);
    println!("XOR加密后: {}", encrypted);
    
    // 测试2: 解密验证
    if let Some(decrypted) = decrypt_legacy_password(&encrypted) {
        println!("解密结果: {}", decrypted);
        println!("✓ XOR加密验证成功\n");
    }
    
    // 测试3: SHA256加密
    let sha256_hash = hash_sha256(test_password);
    println!("SHA256加密: {}\n", sha256_hash);
    
    println!("=== 使用说明 ===");
    println!("1. 从SQL Server查询 tBas_Emp 表中 EmpNo='000064' 的 PassWordStr 字段");
    println!("2. 将查询到的密码值输入到下方进行验证");
    println!("3. 如果是16位十六进制，则是XOR加密");
    println!("4. 如果以SHA256:开头，则是SHA256加密");
    println!("5. 其他情况可能是明文\n");
    
    // 常见测试
    let test_cases = vec!["123456", "admin", "000064", "password"];
    println!("=== 常见密码的XOR加密值 ===");
    for pwd in test_cases {
        println!("{} -> {}", pwd, encrypt_legacy_password(pwd));
    }
}
