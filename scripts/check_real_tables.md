# 数据库表名/字段核查脚本

## 用途

核对 `server-rust/src/**/*.rs` 和 `client/src/**/*.vue` 中使用的表名/字段名是否与 TestERP 库真实结构一致。

## 核查范围

| 类别 | 核查表 | 用途 |
|---|---|---|
| 财务 | tFin_*/tArd_*/tAcc_* | 应收/应付/收付款/现金流 |
| OA | tOA_InfoDetail/tSys_WorkFlow | 通知/工作流（前端错用 tSys_Msg） |
| 系统 | tSys_OperLog/tSys_OperHis | 操作日志 |
| 业务主表 | tSal_*/tPur_*/tStk_* | 字段名核对 |
| 库存三件套 | tStk_Stock/StockYM/StockTranHis | 触发器配套 |
| 安全网 | 4 触发器 + 3 CHECK | 已部署核对 |
| 单据编号 | tSys_Doc* / tDoc* | doc_no 配套 |

## 执行

```bash
# Windows PowerShell
sqlcmd -S localhost -d TestERP -U sa -P sa123456 -i check_real_tables.sql -o check_real_tables.txt

# 或 sqlcmd 交互
sqlcmd -S localhost -d TestERP -U sa -P sa123456
> :r check_real_tables.sql
> GO
```

## 回传

把 `check_real_tables.txt` 完整内容贴回对话，开发助手会自动：
1. 修复 `main.rs` 路由
2. 修复 `client/views/oa/OAWorkflow.vue` / `OANotice.vue` 表名
3. 修复 `client/src/api/index.js` 中错误的财务 API 路径
4. 修复 `finance.rs` 中的 tFin_* 表名引用
5. 修复其他发现的表名/字段不一致
