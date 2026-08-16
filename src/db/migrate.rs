use crate::db::pool::get_pool;

/// 单个迁移定义
struct Migration {
    name: &'static str,
    sql: &'static str,
    /// 关键迁移：失败时 error 级别日志 + 计入 failed_critical；
    /// 非关键迁移：失败时 warn 级别日志，继续执行后续迁移。
    critical: bool,
}

/// 启动时执行一次性数据库迁移（带版本控制 + 事务保护）
///
/// 每个迁移与"记录写入"被包在同一个 SQL Server 事务中：
/// - 成功：迁移 SQL + INSERT 记录 一起 COMMIT
/// - 失败：整个事务 ROLLBACK，记录不会写入，下次启动会重试
///
/// "已生效"类错误（如列已是目标类型）会被识别并标记为已执行，避免反复报错。
pub async fn run_migrations() {
    let pool = get_pool();
    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("迁移：获取数据库连接失败: {}", e);
            return;
        }
    };

    // 确保迁移版本表存在
    let create_table_sql = "
        IF NOT EXISTS (SELECT 1 FROM sys.tables WHERE name = 'tSys_Migration')
        CREATE TABLE tSys_Migration (
            Id INT IDENTITY(1,1) PRIMARY KEY,
            Name NVARCHAR(200) NOT NULL UNIQUE,
            AppliedAt DATETIME NOT NULL DEFAULT GETDATE()
        )";
    if let Err(e) = conn.execute(create_table_sql, &[]).await {
        tracing::error!("创建迁移版本表失败: {}", e);
        return;
    }

    // 迁移列表（按名称顺序执行；新增迁移追加在末尾，不要插入中间）
    let migrations: &[Migration] = &[
        Migration {
            name: "001_config_nvarchar_max",
            sql: "ALTER TABLE tSys_TableColumnConfig ALTER COLUMN ConfigData nvarchar(max) NULL",
            // 列类型变更可能已在早期部署中手工执行，标记为非关键
            critical: false,
        },
        Migration {
            name: "002_preset_nvarchar_max",
            sql: "ALTER TABLE tSys_ColumnPreset ALTER COLUMN ConfigData nvarchar(max) NULL",
            critical: false,
        },
        Migration {
            name: "003_ot_to_oti",
            sql: "UPDATE tStk_IO SET Kind = 'OTI' WHERE Kind = 'OT' AND IONo LIKE 'OTI%'",
            // 数据修正类迁移，重复执行幂等（WHERE 过滤后无匹配行即无操作）
            critical: false,
        },
        Migration {
            name: "004_ot_to_oto",
            sql: "UPDATE tStk_IO SET Kind = 'OTO' WHERE Kind = 'OT' AND IONo LIKE 'OTO%'",
            critical: false,
        },
        // 005: tSys_RuleMenu 新增 CanExport 导出权限位（权限系统完善）
        // 使用 IF NOT EXISTS 避免列已存在时报错（错误码 2705 会让事务状态混乱）
        Migration {
            name: "005_rulemenu_add_can_export",
            sql: "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSys_RuleMenu' AND COLUMN_NAME = 'CanExport') ALTER TABLE tSys_RuleMenu ADD CanExport int DEFAULT 0",
            critical: false,
        },
        // 006: 修正 tSys_RuleMenu 中 CanRead/CanCreate 等字段的历史脏数据
        //   早期 handler 写入 "Y"/"N" 字符串，已被 SQL Server 隐式转换为 int(1/0)。
        //   此迁移将所有非 0 值归一化为 1，NULL/0 保持 0。
        //   注意：不能与字符串 'N' 比较（int 列会报转换错误 245）
        Migration {
            name: "006_rulemenu_normalize_perm_flags",
            sql: "UPDATE tSys_RuleMenu SET CanRead = CASE WHEN CanRead IS NULL OR CanRead = 0 THEN 0 ELSE 1 END, CanCreate = CASE WHEN CanCreate IS NULL OR CanCreate = 0 THEN 0 ELSE 1 END, CanUpdate = CASE WHEN CanUpdate IS NULL OR CanUpdate = 0 THEN 0 ELSE 1 END, CanDelete = CASE WHEN CanDelete IS NULL OR CanDelete = 0 THEN 0 ELSE 1 END, CanAudit = CASE WHEN CanAudit IS NULL OR CanAudit = 0 THEN 0 ELSE 1 END, CanPrint = CASE WHEN CanPrint IS NULL OR CanPrint = 0 THEN 0 ELSE 1 END, CanExport = CASE WHEN CanExport IS NULL OR CanExport = 0 THEN 0 ELSE 1 END",
            critical: false,
        },
        // 007: 重建 tSys_Config 表（适配 SystemConfig.vue + 全局 UI 配置）
        //   旧表只有 ConfigKey/ConfigValue/Remark/EDate/EUser 5 字段，主键 ConfigKey
        //   新表扩展为 ConfigID(uniqueidentifier) 主键 + ConfigGroup/ConfigName/ValueType/SortOrder/Used/Description/CrDate
        //   旧表数据未被任何业务代码使用（后端读公司名用的是 tSys_Parameters），可安全重建
        //   重复执行幂等：检测 ConfigID 列存在则跳过；表不存在时直接创建
        Migration {
            name: "007_rebuild_tsys_config",
            sql: "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSys_Config' AND COLUMN_NAME = 'ConfigID') \
                   BEGIN \
                     IF EXISTS (SELECT * FROM sys.tables WHERE name = 'tSys_Config') \
                     BEGIN \
                       IF NOT EXISTS (SELECT * FROM sys.tables WHERE name = 'tSys_Config_BAK') SELECT * INTO tSys_Config_BAK FROM tSys_Config; \
                       DROP TABLE tSys_Config; \
                     END \
                     CREATE TABLE [tSys_Config] ( \
                       [ConfigID]     UNIQUEIDENTIFIER NOT NULL DEFAULT NEWID() PRIMARY KEY, \
                       [ConfigGroup]  VARCHAR(50)   NOT NULL DEFAULT 'general', \
                       [ConfigName]   NVARCHAR(100) NOT NULL, \
                       [ConfigKey]    VARCHAR(100)  NOT NULL, \
                       [ConfigValue]  NVARCHAR(MAX) NULL, \
                       [ValueType]    VARCHAR(20)   NOT NULL DEFAULT 'string', \
                       [SortOrder]    INT           NOT NULL DEFAULT 0, \
                       [Used]         CHAR(1)       NOT NULL DEFAULT 'Y', \
                       [Description]  NVARCHAR(500) NULL, \
                       [EDate]        DATETIME      NULL, \
                       [EUser]        VARCHAR(20)   NULL, \
                       [CrDate]       DATETIME      NOT NULL DEFAULT GETDATE() \
                     ); \
                     CREATE UNIQUE INDEX UQ_tSys_Config_Key ON tSys_Config(ConfigKey); \
                     CREATE INDEX IX_tSys_Config_Group ON tSys_Config(ConfigGroup); \
                     CREATE INDEX IX_tSys_Config_Used ON tSys_Config(Used); \
                   END",
            critical: false,
        },
        // 008: 修正 tSys_Config.EUser 列长度（VARCHAR(20) → VARCHAR(36)）
        //   generic_create 自动用登录用户的 EmpID（UUID，36 字符）填充 EUser，
        //   007 迁移建表时误设为 VARCHAR(20)，导致 INSERT 报"将截断字符串或二进制数据"。
        //   同时把 ConfigKey 扩到 VARCHAR(100)，避免长 key 被截断（实际已是 100，幂等）。
        Migration {
            name: "008_tsys_config_euser_size",
            sql: "IF EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSys_Config' AND COLUMN_NAME = 'EUser' AND CHARACTER_MAXIMUM_LENGTH < 36) ALTER TABLE tSys_Config ALTER COLUMN EUser VARCHAR(36) NULL",
            critical: false,
        },
        // 009: tArd_AR 补货明细表索引（278万+行）
        //   手机数据 PC 端按 EDate + StkID 分组查询补货单，无索引时全表扫描 14 秒+。
        //   EDate 索引让日期范围下推后的内层 WHERE 快速定位数据。
        Migration {
            name: "009_tard_ar_edate_index",
            sql: "IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tArd_AR_EDate' AND object_id = OBJECT_ID('tArd_AR')) CREATE INDEX IX_tArd_AR_EDate ON tArd_AR(EDate)",
            critical: false,
        },
        // 010: 高频查询字段索引（P4-9）
        //   基于 handlers/ 下 WHERE 条件统计，共 8 类高频字段
        //   关键实现要点（修复首次启动失败的问题）：
        //     1) 每条 CREATE INDEX 必须用 EXEC('...') 包裹，使其成为独立批处理
        //        否则 IF NOT EXISTS (...) CREATE INDEX 后跟下一条 IF 语句时，
        //        SQL Server 会将多条 IF 语句视为同一 batch，触发 "BEGIN TRY 必须是批处理中唯一语句" 错误
        //     2) IF NOT EXISTS 保证幂等：重复执行不会报"已存在"错误
        //     3) EXEC sp_executesql 包裹避免语法解析问题，且动态 SQL 内部错误可被外层 TRY/CATCH 捕获
        //     4) 已生效的"列不存在/表不存在"错误被 is_already_applied 识别为已执行
        //   标记为非关键：整体失败不阻断启动
        Migration {
            name: "010_high_freq_indexes",
            sql: concat!(
                // 1) 单据主表查询：单据号 / 状态
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_IONo' AND object_id = OBJECT_ID('tStk_IO')) EXEC('CREATE INDEX IX_tStk_IO_IONo ON tStk_IO(IONo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IO_State' AND object_id = OBJECT_ID('tStk_IO')) EXEC('CREATE INDEX IX_tStk_IO_State ON tStk_IO(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Move_MoveNo' AND object_id = OBJECT_ID('tStk_Move')) EXEC('CREATE INDEX IX_tStk_Move_MoveNo ON tStk_Move(MoveNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Move_State' AND object_id = OBJECT_ID('tStk_Move')) EXEC('CREATE INDEX IX_tStk_Move_State ON tStk_Move(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Tran_TranNo' AND object_id = OBJECT_ID('tStk_Tran')) EXEC('CREATE INDEX IX_tStk_Tran_TranNo ON tStk_Tran(TranNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Tran_State' AND object_id = OBJECT_ID('tStk_Tran')) EXEC('CREATE INDEX IX_tStk_Tran_State ON tStk_Tran(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_ReplenishApply_No' AND object_id = OBJECT_ID('tStk_ReplenishApply')) EXEC('CREATE INDEX IX_tStk_ReplenishApply_No ON tStk_ReplenishApply(ReplenishApplyNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Order_PoNo' AND object_id = OBJECT_ID('tPur_Order')) EXEC('CREATE INDEX IX_tPur_Order_PoNo ON tPur_Order(PoNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Order_State' AND object_id = OBJECT_ID('tPur_Order')) EXEC('CREATE INDEX IX_tPur_Order_State ON tPur_Order(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_Quote_PqNo' AND object_id = OBJECT_ID('tPur_Quote')) EXEC('CREATE INDEX IX_tPur_Quote_PqNo ON tPur_Quote(PqNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Order_SoNo' AND object_id = OBJECT_ID('tSal_Order')) EXEC('CREATE INDEX IX_tSal_Order_SoNo ON tSal_Order(SoNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_Order_State' AND object_id = OBJECT_ID('tSal_Order')) EXEC('CREATE INDEX IX_tSal_Order_State ON tSal_Order(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_RecNo' AND object_id = OBJECT_ID('tFin_Receipt')) EXEC('CREATE INDEX IX_tFin_Receipt_RecNo ON tFin_Receipt(RecNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Receipt_State' AND object_id = OBJECT_ID('tFin_Receipt')) EXEC('CREATE INDEX IX_tFin_Receipt_State ON tFin_Receipt(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_PayNo' AND object_id = OBJECT_ID('tFin_Payment')) EXEC('CREATE INDEX IX_tFin_Payment_PayNo ON tFin_Payment(PayNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tFin_Payment_State' AND object_id = OBJECT_ID('tFin_Payment')) EXEC('CREATE INDEX IX_tFin_Payment_State ON tFin_Payment(State)') END TRY BEGIN CATCH END CATCH;",
                // 2) 明细表外键
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_IODetail_IOID' AND object_id = OBJECT_ID('tStk_IODetail')) EXEC('CREATE INDEX IX_tStk_IODetail_IOID ON tStk_IODetail(IOID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_MoveDetail_MoveID' AND object_id = OBJECT_ID('tStk_MoveDetail')) EXEC('CREATE INDEX IX_tStk_MoveDetail_MoveID ON tStk_MoveDetail(MoveID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_TranDetail_TranID' AND object_id = OBJECT_ID('tStk_TranDetail')) EXEC('CREATE INDEX IX_tStk_TranDetail_TranID ON tStk_TranDetail(TranID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_OrderDetail_POID' AND object_id = OBJECT_ID('tPur_OrderDetail')) EXEC('CREATE INDEX IX_tPur_OrderDetail_POID ON tPur_OrderDetail(POID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tPur_QuoteDetail_PQID' AND object_id = OBJECT_ID('tPur_QuoteDetail')) EXEC('CREATE INDEX IX_tPur_QuoteDetail_PQID ON tPur_QuoteDetail(PQID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSal_OrderDetail_SOID' AND object_id = OBJECT_ID('tSal_OrderDetail')) EXEC('CREATE INDEX IX_tSal_OrderDetail_SOID ON tSal_OrderDetail(SOID)') END TRY BEGIN CATCH END CATCH;",
                // 3) 权限系统：RuleID / EmpID 反向查询
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_RuleMenu_RuleID' AND object_id = OBJECT_ID('tSys_RuleMenu')) EXEC('CREATE INDEX IX_tSys_RuleMenu_RuleID ON tSys_RuleMenu(RuleID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_UserRule_EmpID' AND object_id = OBJECT_ID('tSys_UserRule')) EXEC('CREATE INDEX IX_tSys_UserRule_EmpID ON tSys_UserRule(EmpID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_UserRule_RuleID' AND object_id = OBJECT_ID('tSys_UserRule')) EXEC('CREATE INDEX IX_tSys_UserRule_RuleID ON tSys_UserRule(RuleID)') END TRY BEGIN CATCH END CATCH;",
                // 4) 系统参数 / 通知
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_Parameters_PCode' AND object_id = OBJECT_ID('tSys_Parameters')) EXEC('CREATE INDEX IX_tSys_Parameters_PCode ON tSys_Parameters(PCode)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_Parameters_PKind' AND object_id = OBJECT_ID('tSys_Parameters')) EXEC('CREATE INDEX IX_tSys_Parameters_PKind ON tSys_Parameters(PKind)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_Msg_TEmpID_State' AND object_id = OBJECT_ID('tSys_Msg')) EXEC('CREATE INDEX IX_tSys_Msg_TEmpID_State ON tSys_Msg(TEmpID, State)') END TRY BEGIN CATCH END CATCH;",
                // 5) 操作日志：高频关联查询（P4-9 重点）
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperLog_KeyValue' AND object_id = OBJECT_ID('tSys_OperLog')) EXEC('CREATE INDEX IX_tSys_OperLog_KeyValue ON tSys_OperLog(KeyValue)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperLog_OperDate' AND object_id = OBJECT_ID('tSys_OperLog')) EXEC('CREATE INDEX IX_tSys_OperLog_OperDate ON tSys_OperLog(OperDate)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperLog_TableName' AND object_id = OBJECT_ID('tSys_OperLog')) EXEC('CREATE INDEX IX_tSys_OperLog_TableName ON tSys_OperLog(TableName)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperHis_DocID' AND object_id = OBJECT_ID('tSys_OperHis')) EXEC('CREATE INDEX IX_tSys_OperHis_DocID ON tSys_OperHis(DocID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_OperHis_OperDate' AND object_id = OBJECT_ID('tSys_OperHis')) EXEC('CREATE INDEX IX_tSys_OperHis_OperDate ON tSys_OperHis(OperDate)') END TRY BEGIN CATCH END CATCH;",
                // 6) 客户定价 / 上传文件
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_CustPriceTac_CustID_BrandID' AND object_id = OBJECT_ID('tBas_CustPriceTac')) EXEC('CREATE INDEX IX_tBas_CustPriceTac_CustID_BrandID ON tBas_CustPriceTac(CustID, BrandID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tSys_UploadFile_BizType_BizID' AND object_id = OBJECT_ID('tSys_UploadFile')) EXEC('CREATE INDEX IX_tSys_UploadFile_BizType_BizID ON tSys_UploadFile(BizType, BizID)') END TRY BEGIN CATCH END CATCH;",
                // 7) 库存查询
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Stock_GDSID' AND object_id = OBJECT_ID('tStk_Stock')) EXEC('CREATE INDEX IX_tStk_Stock_GDSID ON tStk_Stock(GDSID)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tStk_Stock_StkID' AND object_id = OBJECT_ID('tStk_Stock')) EXEC('CREATE INDEX IX_tStk_Stock_StkID ON tStk_Stock(StkID)') END TRY BEGIN CATCH END CATCH;",
                // 8) 基础资料：编码 + 状态
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Goods_GDSNO' AND object_id = OBJECT_ID('tBas_Goods')) EXEC('CREATE INDEX IX_tBas_Goods_GDSNO ON tBas_Goods(GDSNO)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Goods_State' AND object_id = OBJECT_ID('tBas_Goods')) EXEC('CREATE INDEX IX_tBas_Goods_State ON tBas_Goods(State)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Supp_SuppNo' AND object_id = OBJECT_ID('tBas_Supp')) EXEC('CREATE INDEX IX_tBas_Supp_SuppNo ON tBas_Supp(SuppNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Cust_CustNo' AND object_id = OBJECT_ID('tBas_Cust')) EXEC('CREATE INDEX IX_tBas_Cust_CustNo ON tBas_Cust(CustNo)') END TRY BEGIN CATCH END CATCH;",
                "BEGIN TRY IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = 'IX_tBas_Emp_EmpNo' AND object_id = OBJECT_ID('tBas_Emp')) EXEC('CREATE INDEX IX_tBas_Emp_EmpNo ON tBas_Emp(EmpNo)') END TRY BEGIN CATCH END CATCH;",
            ),
            critical: false,
        },
        // 011: 关键表主键 DEFAULT NEWID() 安全网
        //   背景：tStk_IO.IOID / tStk_Move.MoveID / tStk_Tran.TranID / tStk_ReplenishApply.ReplenishApplyID
        //   在 INSERT 时虽已显式 NEWID()，但表本身无默认约束。
        //   若未来有 INSERT 漏写 NEWID()（如 generic/create），数据库会直接拒绝（NOT NULL 无默认）。
        //   本迁移仅作安全网，幂等：仅当列存在且无默认约束时添加
        Migration {
            name: "011_safe_default_pks",
            sql: concat!(
                // tStk_IO.IOID
                "IF OBJECT_ID('tStk_IO', 'U') IS NOT NULL AND COL_LENGTH('tStk_IO', 'IOID') IS NOT NULL ",
                "AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_IO') AND c.name = 'IOID') ",
                "ALTER TABLE [tStk_IO] ADD CONSTRAINT [DF_tStk_IO_IOID] DEFAULT NEWID() FOR [IOID];",
                // tStk_Move.MoveID
                "IF OBJECT_ID('tStk_Move', 'U') IS NOT NULL AND COL_LENGTH('tStk_Move', 'MoveID') IS NOT NULL ",
                "AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Move') AND c.name = 'MoveID') ",
                "ALTER TABLE [tStk_Move] ADD CONSTRAINT [DF_tStk_Move_MoveID] DEFAULT NEWID() FOR [MoveID];",
                // tStk_Tran.TranID
                "IF OBJECT_ID('tStk_Tran', 'U') IS NOT NULL AND COL_LENGTH('tStk_Tran', 'TranID') IS NOT NULL ",
                "AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_Tran') AND c.name = 'TranID') ",
                "ALTER TABLE [tStk_Tran] ADD CONSTRAINT [DF_tStk_Tran_TranID] DEFAULT NEWID() FOR [TranID];",
                // tStk_ReplenishApply.ReplenishApplyID
                "IF OBJECT_ID('tStk_ReplenishApply', 'U') IS NOT NULL AND COL_LENGTH('tStk_ReplenishApply', 'ReplenishApplyID') IS NOT NULL ",
                "AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_ReplenishApply') AND c.name = 'ReplenishApplyID') ",
                "ALTER TABLE [tStk_ReplenishApply] ADD CONSTRAINT [DF_tStk_ReplenishApply_ID] DEFAULT NEWID() FOR [ReplenishApplyID];",
                // tStk_StockCycle.StkCycleID (周期盘点主表)
                "IF OBJECT_ID('tStk_StockCycle', 'U') IS NOT NULL AND COL_LENGTH('tStk_StockCycle', 'StkCycleID') IS NOT NULL ",
                "AND NOT EXISTS (SELECT 1 FROM sys.default_constraints dc JOIN sys.columns c ON dc.parent_object_id = c.object_id AND dc.parent_column_id = c.column_id WHERE c.object_id = OBJECT_ID('tStk_StockCycle') AND c.name = 'StkCycleID') ",
                "ALTER TABLE [tStk_StockCycle] ADD CONSTRAINT [DF_tStk_StockCycle_ID] DEFAULT NEWID() FOR [StkCycleID];",
            ),
            // D6 修复：主键默认值是数据完整性关键，缺失会导致 INSERT 失败
            //   标记 critical: true，失败时 error 级别日志便于发现
            critical: true,
        },
        // 012: D1 创建 admin 用户（critical: true）
        //   - 新部署的系统必须有 admin 账号才能登录
        //   - 幂等：IF NOT EXISTS 检查 EmpNo='admin'
        //   - 密码使用 SHA256+静态盐格式（兼容 verify_password 旧格式）
        //     首次登录后会被自动升级为 bcrypt 格式（更安全）
        //   - EmpID 使用固定 UUID，便于 D2 中将 admin 分配到默认角色
        Migration {
            name: "012_seed_admin_user",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM [tBas_Emp] WHERE [EmpNo] = N'admin') ",
                "BEGIN ",
                "  INSERT INTO [tBas_Emp] ",
                "  ([EmpID], [EmpNo], [EmpName], [Sex], [WorkState], [AllowLogin], [State], [Note], ",
                "   [EUser], [EDate], [InDate], [PYCode], ",
                "   [PassWordStr]) ",
                "  VALUES (",
                "    '00000000-0000-1000-0000-000000000001', N'admin', N'系统管理员', 1, N'1', N'Y', N'Y', ",
                "    N'系统初始管理员账号（migration 012_seed_admin_user 自动创建）', ",
                "    '00000000-0000-1000-0000-000000000001', GETDATE(), GETDATE(), N'admin', ",
                "    N'SHA256:eeaeaefac2f357bc46a8337007e3d0472e36795c2c6b68114c4585f43d97cb60'",
                "  ); ",
                "END"
            ),
            critical: true,
        },
        // 013: D2 创建默认角色 + 分配所有菜单权限给"系统管理员" + 分配 admin 用户到"系统管理员"
        //   - critical: true：缺失角色会导致非 admin 用户登录后没有任何权限
        //   - 幂等：所有 INSERT 都用 IF NOT EXISTS / NOT EXISTS 检查
        //   - 注：BEGIN/END 块在单条 SQL 中可以包含多个语句，但 tiberius 单次 execute
        //     只执行第一条语句。这里用多个独立 IF 块（;）+ INSERT...SELECT WHERE NOT EXISTS 模式
        Migration {
            name: "013_seed_default_roles",
            sql: concat!(
                // 1. 创建"系统管理员"角色
                "IF NOT EXISTS (SELECT 1 FROM [tSys_Rule] WHERE [RuleID] = '10000000-0000-1000-0000-000000000001') ",
                "INSERT INTO [tSys_Rule] ([RuleID], [RuleName], [Note], [Flg], [State]) ",
                "VALUES ('10000000-0000-1000-0000-000000000001', N'系统管理员', N'系统初始化角色，拥有所有菜单的所有权限', N'admin', N'Y'); ",
                // 2. 创建"普通用户"角色
                "IF NOT EXISTS (SELECT 1 FROM [tSys_Rule] WHERE [RuleID] = '10000000-0000-1000-0000-000000000002') ",
                "INSERT INTO [tSys_Rule] ([RuleID], [RuleName], [Note], [Flg], [State]) ",
                "VALUES ('10000000-0000-1000-0000-000000000002', N'普通用户', N'系统初始化角色，仅对所有菜单有只读权限', N'user', N'Y'); ",
                // 3. 系统管理员：分配所有菜单的全部权限（仅插入不存在的）
                "INSERT INTO [tSys_RuleMenu] ",
                "  ([RuleMenuID], [RuleID], [MenuID], [CanRead], [CanCreate], [CanUpdate], [CanDelete], [CanAudit], [CanPrint], [CanExport], [LUTime]) ",
                "SELECT NEWID(), '10000000-0000-1000-0000-000000000001', m.[SYM_ID], 1, 1, 1, 1, 1, 1, 1, GETDATE() ",
                "FROM [tSys_Menus] m ",
                "WHERE ISNULL(m.[Used], 'Y') = 'Y' AND m.[SYM_ID] IS NOT NULL ",
                "  AND NOT EXISTS (SELECT 1 FROM [tSys_RuleMenu] rm WHERE rm.[RuleID] = '10000000-0000-1000-0000-000000000001' AND rm.[MenuID] = m.[SYM_ID]); ",
                // 4. 普通用户：分配所有菜单的只读权限
                "INSERT INTO [tSys_RuleMenu] ",
                "  ([RuleMenuID], [RuleID], [MenuID], [CanRead], [CanCreate], [CanUpdate], [CanDelete], [CanAudit], [CanPrint], [CanExport], [LUTime]) ",
                "SELECT NEWID(), '10000000-0000-1000-0000-000000000002', m.[SYM_ID], 1, 0, 0, 0, 0, 0, 0, GETDATE() ",
                "FROM [tSys_Menus] m ",
                "WHERE ISNULL(m.[Used], 'Y') = 'Y' AND m.[SYM_ID] IS NOT NULL ",
                "  AND NOT EXISTS (SELECT 1 FROM [tSys_RuleMenu] rm WHERE rm.[RuleID] = '10000000-0000-1000-0000-000000000002' AND rm.[MenuID] = m.[SYM_ID]); ",
                // 5. 将 admin 用户分配到"系统管理员"角色
                "IF NOT EXISTS (",
                "  SELECT 1 FROM [tSys_UserRule] ur ",
                "  INNER JOIN [tBas_Emp] e ON ur.[EmpID] = e.[EmpID] ",
                "  WHERE e.[EmpNo] = N'admin' AND ur.[RuleID] = '10000000-0000-1000-0000-000000000001') ",
                "INSERT INTO [tSys_UserRule] ([UserRuleID], [EmpID], [RuleID], [LUTime]) ",
                "SELECT NEWID(), e.[EmpID], '10000000-0000-1000-0000-000000000001', GETDATE() ",
                "FROM [tBas_Emp] e WHERE e.[EmpNo] = N'admin';",
            ),
            critical: true,
        },
        // 014: 用户偏好表 tSys_UserPref（按用户持久化主题、布局等偏好，支持跨设备同步）
        //   - EmpID + PrefKey 唯一索引，保证一个用户同一偏好只有一行
        //   - PrefKey/PrefValue 设计通用化，未来可保存布局密度、默认仓库等
        //   - 幂等：IF NOT EXISTS 检查表是否存在
        //   - critical: false：表缺失不影响登录，前端会回退到 localStorage
        Migration {
            name: "014_create_user_pref_table",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM sys.tables WHERE name = 'tSys_UserPref') ",
                "BEGIN ",
                "  CREATE TABLE [tSys_UserPref] ( ",
                "    [UserPrefID] UNIQUEIDENTIFIER NOT NULL CONSTRAINT [DF_tSys_UserPref_UserPrefID] DEFAULT NEWID(), ",
                "    [EmpID] UNIQUEIDENTIFIER NOT NULL, ",
                "    [PrefKey] NVARCHAR(64) NOT NULL, ",
                "    [PrefValue] NVARCHAR(255) NULL, ",
                "    [LUTime] DATETIME NOT NULL CONSTRAINT [DF_tSys_UserPref_LUTime] DEFAULT GETDATE(), ",
                "    CONSTRAINT [PK_tSys_UserPref] PRIMARY KEY ([UserPrefID]) ",
                "  ); ",
                "  CREATE UNIQUE INDEX [UX_tSys_UserPref_EmpID_PrefKey] ON [tSys_UserPref]([EmpID], [PrefKey]); ",
                "END",
            ),
            critical: false,
        },
        // 015: tBas_Stock 补 EUser 字段（创建人 EmpID），与其他基础资料表对齐
        //   - 背景：tBas_Stock 原表无 EUser 列，无法记录创建人，列表也无法显示创建人
        //   - generic_create 会通过 has_column 检测后自动填充当前登录用户 EmpID
        //   - 幂等：IF NOT EXISTS 检查列是否存在
        //   - critical: false：列已存在时 ALTER 不执行，无副作用
        Migration {
            name: "015_tbas_stock_add_euser",
            sql: "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tBas_Stock' AND COLUMN_NAME = 'EUser') ALTER TABLE [tBas_Stock] ADD [EUser] VARCHAR(36) NULL",
            critical: false,
        },
        // 016: 创建缺货记录表 tStk_Shortage
        //   - 背景：用户保存/审核单据时库存不足（ApproveError::Shortage），原实现只把 shortage_list
        //     返回前端弹窗展示，不持久化。采购员无法看到历史缺货数据，需要人工补货或导出
        //   - 本表持久化每条缺货明细：来源单据、商品、仓库、需求量、不足量、当时的库存/预占快照
        //   - State 字段语义：N=未处理，S=已转采购订单，D=已删除（与单据状态语义一致）
        //   - 索引：GDSID+StkID（按商品+仓库查询）、EDate（按时间排序）、State（按状态过滤）
        Migration {
            name: "016_create_tstk_shortage",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM sys.tables WHERE name = 'tStk_Shortage') ",
                "BEGIN ",
                "  CREATE TABLE [tStk_Shortage] ( ",
                "    [ShortageID]      UNIQUEIDENTIFIER NOT NULL CONSTRAINT [DF_tStk_Shortage_ID] DEFAULT NEWID(), ",
                "    [GDSID]           UNIQUEIDENTIFIER NOT NULL, ",
                "    [StkID]           UNIQUEIDENTIFIER NOT NULL, ",
                "    [Qty]             DECIMAL(18,4) NOT NULL DEFAULT 0, ",
                "    [ShortQty]        DECIMAL(18,4) NOT NULL DEFAULT 0, ",
                "    [StockQty]        DECIMAL(18,4) NOT NULL DEFAULT 0, ",
                "    [ReservedQty]     DECIMAL(18,4) NOT NULL DEFAULT 0, ",
                "    [SourceDocTable]  NVARCHAR(50)  NULL, ",
                "    [SourceDocNo]     NVARCHAR(50)  NULL, ",
                "    [SourceDocID]     UNIQUEIDENTIFIER NULL, ",
                "    [SourceKind]      NVARCHAR(20)  NOT NULL DEFAULT 'doc_save', ",
                "    [Remark]          NVARCHAR(500) NULL, ",
                "    [EUser]           NVARCHAR(50)  NOT NULL, ",
                "    [EmpID]           UNIQUEIDENTIFIER NOT NULL, ",
                "    [EDate]           DATETIME      NOT NULL CONSTRAINT [DF_tStk_Shortage_EDate] DEFAULT GETDATE(), ",
                "    [State]           NCHAR(1)      NOT NULL CONSTRAINT [DF_tStk_Shortage_State] DEFAULT 'N', ",
                "    [LUTime]         DATETIME      NULL, ",
                "    [LUUser]         NVARCHAR(50)  NULL, ",
                "    CONSTRAINT [PK_tStk_Shortage] PRIMARY KEY ([ShortageID]) ",
                "  ); ",
                "  CREATE INDEX [IX_tStk_Shortage_GDSStk] ON [tStk_Shortage]([GDSID], [StkID]); ",
                "  CREATE INDEX [IX_tStk_Shortage_EDate]  ON [tStk_Shortage]([EDate]); ",
                "  CREATE INDEX [IX_tStk_Shortage_State]  ON [tStk_Shortage]([State]); ",
                "  CREATE INDEX [IX_tStk_Shortage_Source] ON [tStk_Shortage]([SourceDocID]); ",
                "END",
            ),
            // 表缺失会导致缺货记录无法持久化，采购员看不到缺货数据
            critical: true,
        },
        // 017: 为 tStk_Shortage 增加"缺货对象"字段（CustID + CustName + ShopID + ShopName）
        //   - CustID/CustName：缺货客户（销售类单据的客户，来自 tBas_Cust）
        //   - ShopID/ShopName：要货门店（门店直配 ZP 的调入仓，来自 tBas_Stock NodeKind='C'）
        //   - 两者分开存储，便于按客户或门店独立筛选查询
        //   - 索引：CustID + ShopID（分别按客户/门店统计缺货）
        Migration {
            name: "017_tstk_shortage_add_cust",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tStk_Shortage' AND COLUMN_NAME = 'CustID') ",
                "BEGIN ",
                "  ALTER TABLE [tStk_Shortage] ADD [CustID] UNIQUEIDENTIFIER NULL; ",
                "  ALTER TABLE [tStk_Shortage] ADD [CustName] NVARCHAR(100) NULL; ",
                "  ALTER TABLE [tStk_Shortage] ADD [ShopID] UNIQUEIDENTIFIER NULL; ",
                "  ALTER TABLE [tStk_Shortage] ADD [ShopName] NVARCHAR(100) NULL; ",
                "  CREATE INDEX [IX_tStk_Shortage_CustID] ON [tStk_Shortage]([CustID]); ",
                "  CREATE INDEX [IX_tStk_Shortage_ShopID] ON [tStk_Shortage]([ShopID]); ",
                "END",
            ),
            critical: true,
        },
        // 018: 客户表增加定价模板关联字段
        //   - PricingTemplateID 关联 tSys_Parameters.ParametersID（PKind='pricing'）
        //   - 一个客户可绑定一个定价模板；NULL 表示无专属模板，走默认零售价
        //   - 参考 88 文件 customers.PricingTemplateID 方案
        Migration {
            name: "018_tbas_cust_add_pricing_template",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tBas_Cust' AND COLUMN_NAME = 'PricingTemplateID') ",
                "BEGIN ",
                "  ALTER TABLE [tBas_Cust] ADD [PricingTemplateID] UNIQUEIDENTIFIER NULL; ",
                "  CREATE INDEX [IX_tBas_Cust_PricingTemplateID] ON [tBas_Cust]([PricingTemplateID]); ",
                "END",
            ),
            critical: true,
        },
        // 019: 仓库表增加提成模板关联字段
        //   - CommissionTemplateID 关联 tSys_Parameters.ParametersID（PKind='commission'）
        //   - 每个门店可挂一个提成模板；NULL 表示无专属模板
        //   - 参考 88 文件 warehouses.commission_template_id 方案
        Migration {
            name: "019_tbas_stock_add_commission_template",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tBas_Stock' AND COLUMN_NAME = 'CommissionTemplateID') ",
                "BEGIN ",
                "  ALTER TABLE [tBas_Stock] ADD [CommissionTemplateID] UNIQUEIDENTIFIER NULL; ",
                "  CREATE INDEX [IX_tBas_Stock_CommissionTemplateID] ON [tBas_Stock]([CommissionTemplateID]); ",
                "END",
            ),
            critical: true,
        },
        // 020: 销售单明细表增加提成字段
        //   - CommissionRate  提成比例（小数，0.12=12%）
        //   - CommissionType  提成类型（0=无, 1=商品规则, 2=品牌规则）
        //   - Commission      提成金额（= Amt × CommissionRate）
        //   - 保存销售单时由后端自动计算并写入，报表直接聚合
        Migration {
            name: "020_tsal_invdetail_add_commission",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSal_InvDetail' AND COLUMN_NAME = 'CommissionRate') ",
                "BEGIN ",
                "  ALTER TABLE [tSal_InvDetail] ADD [CommissionRate] DECIMAL(10,4) DEFAULT 0; ",
                "  ALTER TABLE [tSal_InvDetail] ADD [CommissionType] INT DEFAULT 0; ",
                "  ALTER TABLE [tSal_InvDetail] ADD [Commission] DECIMAL(18,2) DEFAULT 0; ",
                "END",
            ),
            critical: true,
        },
        // 021: 品牌表增加等级字段（A/B/C/D），用于提成报表筛选和分组
        //   对齐 88 项目 brands.level 字段
        Migration {
            name: "021_tbas_brand_add_level",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tBas_Brand' AND COLUMN_NAME = 'Level') ",
                "BEGIN ",
                "  ALTER TABLE [tBas_Brand] ADD [Level] NVARCHAR(50) NULL; ",
                "END",
            ),
            critical: false,
        },
        // 022: 销售单主表增加总提成字段，用于列表显示
        //   对齐 88 项目 store_sales_orders.total_commission 字段
        Migration {
            name: "022_tsal_inv_add_total_commission",
            sql: concat!(
                "IF NOT EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'tSal_Inv' AND COLUMN_NAME = 'TotalCommission') ",
                "BEGIN ",
                "  ALTER TABLE [tSal_Inv] ADD [TotalCommission] DECIMAL(18,2) DEFAULT 0; ",
                "END",
            ),
            critical: false,
        },
    ];

    let mut applied = 0i32;
    let mut skipped = 0i32;
    let mut failed_critical = 0i32;
    let mut failed_non_critical = 0i32;

    for m in migrations {
        // 用 EXISTS 替代 COUNT，更高效
        let check_sql = "SELECT CASE WHEN EXISTS (SELECT 1 FROM tSys_Migration WHERE Name = @p1) \
             THEN 1 ELSE 0 END";
        let exists: i32 = match conn.query(check_sql, &[&m.name.to_string()]).await {
            Ok(stream) => stream
                .into_row()
                .await
                .ok()
                .flatten()
                .and_then(|row| row.get::<i32, _>(0))
                .unwrap_or(0),
            Err(e) => {
                tracing::warn!("迁移检查失败 [{}]: {}", m.name, e);
                if m.critical {
                    failed_critical += 1;
                }
                continue;
            }
        };
        if exists == 1 {
            skipped += 1;
            continue;
        }

        // 将迁移 SQL 与记录写入包在同一个事务中（TRY/CATCH 自动回滚）
        // 占位符 @P1 由 tiberius 自动绑定（@p1 在 tiberius 中等价）
        let wrapped = format!(
            "BEGIN TRY\n\
             \x20 BEGIN TRANSACTION;\n\
             \x20 {migration_sql};\n\
             \x20 INSERT INTO tSys_Migration (Name) VALUES (@P1);\n\
             \x20 COMMIT TRANSACTION;\n\
             \x20 SELECT 1 AS ok, CAST(NULL AS NVARCHAR(4000)) AS msg;\n\
             END TRY\n\
             BEGIN CATCH\n\
             \x20 IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;\n\
             \x20 SELECT 0 AS ok, ERROR_MESSAGE() AS msg;\n\
             END CATCH",
            migration_sql = m.sql
        );

        // 先把查询结果提取到栈上（Option<(ok, msg)>），stream 随即 drop，
        // 释放对 conn 的借用，之后才能再次借用 conn 做"补写记录"。
        let result: Option<(i32, Option<String>)> =
            match conn.query(&wrapped, &[&m.name.to_string()]).await {
                Ok(stream) => stream.into_row().await.ok().flatten().map(|row| {
                    let ok: i32 = row.get::<i32, _>("ok").unwrap_or(0);
                    let msg: Option<String> = row.get::<&str, _>("msg").map(|s| s.to_string());
                    (ok, msg)
                }),
                Err(e) => Some((0, Some(e.to_string()))),
            };

        match result {
            Some((1, _)) => {
                applied += 1;
                tracing::info!("迁移成功: {}", m.name);
            }
            Some((_, Some(msg))) if is_already_applied(&msg) => {
                let _ = conn
                    .execute(
                        "INSERT INTO tSys_Migration (Name) VALUES (@p1)",
                        &[&m.name.to_string()],
                    )
                    .await;
                skipped += 1;
                tracing::debug!("迁移跳过（已生效）: {} - {}", m.name, msg);
            }
            Some((_, Some(msg))) => {
                if m.critical {
                    failed_critical += 1;
                    tracing::error!("迁移失败（关键）: {} - {}", m.name, msg);
                } else {
                    failed_non_critical += 1;
                    tracing::warn!("迁移失败（非关键）: {} - {}", m.name, msg);
                }
            }
            Some((_, None)) => {
                failed_non_critical += 1;
                tracing::warn!("迁移无错误消息 [{}]: 视为失败", m.name);
            }
            None => {
                failed_non_critical += 1;
                tracing::warn!("迁移无返回行 [{}]: 视为失败", m.name);
            }
        };
    }

    tracing::info!(
        "迁移汇总：新增 {}，跳过 {}，关键失败 {}，非关键失败 {}",
        applied,
        skipped,
        failed_critical,
        failed_non_critical
    );

    if failed_critical > 0 {
        tracing::error!(
            "有 {} 个关键迁移失败，建议人工排查 tSys_Migration 表与对应 SQL 后重启服务",
            failed_critical
        );
    }
}

/// 判断错误消息是否表示"迁移已生效"（如列已是目标类型、对象已存在等）
/// 这类错误不视为真正的失败，而是标记为已执行以避免反复报错。
fn is_already_applied(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    // 中英文常见"已生效"信号
    lower.contains("已经是")
        || lower.contains("already")
        || lower.contains("already an object")
        || lower.contains("already exists")
        || lower.contains("duplicate column name")
        || lower.contains("there is already an object named")
        || lower.contains("column names in each table must be unique")
        // 中文错误码 2705：列已存在
        || lower.contains("各表中的列名必须唯一")
        || lower.contains("多次指定了列名")
}
