#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""为 system.rs 的 update_user / delete_user 添加权限缓存失效调用"""

import sys
from pathlib import Path

file_path = Path(r"c:\Users\Administrator\Desktop\ERP\server-rust\src\handlers\system.rs")
content = file_path.read_text(encoding="utf-8")

# 1. update_user: 在密码更新块后、Json(ApiResponse::msg(...)) 前插入缓存失效代码
old_update = '''                    let _ = conn.execute(up, &[a, b, c]).await;
                }
            }
            Json(ApiResponse::msg('''

new_update = '''                    let _ = conn.execute(up, &[a, b, c]).await;
                }
            }
            // 清除该用户的权限缓存（EmpID/RuleID 可能变更）
            if !emp_id.is_empty() {
                invalidate_user_permission_cache(&emp_id);
            }
            // 无法确定旧 EmpID，清除全部缓存以保证一致性
            invalidate_all_permission_cache();
            Json(ApiResponse::msg('''

if old_update in content:
    content = content.replace(old_update, new_update, 1)
    print("OK: update_user 缓存失效已添加")
else:
    print("FAIL: update_user 未找到匹配")
    sys.exit(1)

# 2. delete_user: 在 match 前插入缓存失效（避免破坏 match 语法）
# 原始: let v: &dyn tiberius::ToSql = &body.UserID;
#       match conn.execute(sql, &[v]).await {
# 改为: let v: &dyn tiberius::ToSql = &body.UserID;
#       let _ = conn.execute(sql, &[v]).await;
#       invalidate_all_permission_cache();
#       Json(ApiResponse::msg("..."))
# 但这样会改变返回逻辑，不行。

# 更好的方式：在 match 前先调用 invalidate_all_permission_cache()
old_delete_match = '''    let sql = "UPDATE tSys_User SET State = 'N', LUTime = GETDATE() WHERE UserID = @p1"
    let v: &dyn tiberius::ToSql = &body.UserID;
    match conn.execute(sql, &[v]).await {'''

new_delete_match = '''    let sql = "UPDATE tSys_User SET State = 'N', LUTime = GETDATE() WHERE UserID = @p1"
    let v: &dyn tiberius::ToSql = &body.UserID;
    let result = conn.execute(sql, &[v]).await;
    // 用户被停用后清除权限缓存，避免缓存的权限继续生效
    invalidate_all_permission_cache();
    match result {'''

if old_delete_match in content:
    content = content.replace(old_delete_match, new_delete_match, 1)
    print("OK: delete_user 缓存失效已添加")
else:
    print("FAIL: delete_user 未找到匹配")
    sys.exit(1)

file_path.write_text(content, encoding="utf-8")
print("文件已保存")
