#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
诊断000064登录问题的完整脚本
"""

import subprocess
import sys

print("=" * 70)
print("诊断000064登录问题")
print("=" * 70)
print()

# 1. 检查Rust代码是否重新编译
print("1. 检查编译状态")
print("-" * 70)
try:
    result = subprocess.run(
        ["cargo", "build", "--release", "--message-format=short"],
        cwd=r"c:\Users\Administrator\Desktop\ERP\server-rust",
        capture_output=True,
        text=True,
        timeout=10
    )
    if "Finished" in result.stderr or "Compiling" in result.stderr:
        print("✓ Rust代码已编译")
    else:
        print("⚠ 编译状态未知")
    print(f"编译输出: {result.stderr[:200]}")
except Exception as e:
    print(f"✗ 编译检查失败: {e}")
print()

# 2. 显示需要手动执行的步骤
print("2. 手动操作步骤")
print("-" * 70)
print("请按顺序执行以下步骤：")
print()
print("步骤1: 停止所有erp_server进程")
print("  命令: taskkill /F /IM erp_server.exe")
print("  或者: 打开任务管理器，找到erp_server.exe，结束任务")
print()
print("步骤2: 重新编译")
print("  命令: cd c:\\Users\\Administrator\\Desktop\\ERP\\server-rust")
print("  命令: cargo build --release")
print()
print("步骤3: 启动后端")
print("  命令: c:\\Users\\Administrator\\Desktop\\ERP\\server-rust\\target\\release\\erp_server.exe")
print("  注意: 这个窗口不要关闭！")
print()
print("步骤4: 测试登录")
print("  用浏览器打开: http://localhost:5173")
print("  输入: 000064 / 123456")
print("  观察后端窗口的输出")
print()

# 3. 可能的原因
print("3. 可能的原因分析")
print("-" * 70)
print("✓ 已修复: O/0替换逻辑")
print("✓ 已修复: EmpID安全获取")
print("? 可能问题: PassWordStr字段为NULL导致panic")
print("? 可能问题: 密码升级时SQL执行失败")
print("? 可能问题: 其他字段类型不匹配")
print()

print("=" * 70)
print("请按照上述步骤操作，并告诉我后端窗口的输出")
print("=" * 70)
