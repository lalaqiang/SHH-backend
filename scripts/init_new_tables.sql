-- =============================================
-- 深华辉日化 ERP - 新增系统表建表脚本
-- 适用于 SQL Server 2005+
-- =============================================

-- 打印模板表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_PrintTemplate' AND xtype = 'U')
CREATE TABLE [tSys_PrintTemplate] (
    [TemplateID]      VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateName]    NVARCHAR(100) NOT NULL DEFAULT '',
    [DocType]         VARCHAR(30)  NOT NULL DEFAULT '',
    [PaperWidth]      FLOAT        NOT NULL DEFAULT 210,
    [PaperHeight]     FLOAT        NOT NULL DEFAULT 297,
    [MarginTop]       FLOAT        NOT NULL DEFAULT 10,
    [MarginBottom]    FLOAT        NOT NULL DEFAULT 10,
    [MarginLeft]      FLOAT        NOT NULL DEFAULT 10,
    [MarginRight]     FLOAT        NOT NULL DEFAULT 10,
    [Orientation]     VARCHAR(20)  NOT NULL DEFAULT 'portrait',
    [Content]         NTEXT        NULL,
    [IsDefault]       CHAR(1)      NOT NULL DEFAULT 'N',
    [Remark]          NVARCHAR(200) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 打印配置表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_PrintConfig' AND xtype = 'U')
CREATE TABLE [tSys_PrintConfig] (
    [DocType]         VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateID]      VARCHAR(30)  NULL,
    [Copies]          INT          NOT NULL DEFAULT 1,
    [AutoPrint]       CHAR(1)      NOT NULL DEFAULT 'N',
    [PrinterName]     NVARCHAR(100) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 打印日志表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_PrintLog' AND xtype = 'U')
CREATE TABLE [tSys_PrintLog] (
    [LogID]           VARCHAR(30)  NOT NULL PRIMARY KEY,
    [DocType]         VARCHAR(30)  NOT NULL DEFAULT '',
    [DocNo]           VARCHAR(30)  NOT NULL DEFAULT '',
    [TemplateID]      VARCHAR(30)  NULL,
    [Copies]          INT          NOT NULL DEFAULT 1,
    [PrinterName]     NVARCHAR(100) NULL,
    [PrintUser]       VARCHAR(20)  NULL,
    [PrintDate]       DATETIME     NULL
);

-- 打印版本表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_PrintVersion' AND xtype = 'U')
CREATE TABLE [tSys_PrintVersion] (
    [VersionID]       VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateID]      VARCHAR(30)  NOT NULL,
    [VersionNo]       INT          NOT NULL DEFAULT 1,
    [Content]         NTEXT        NULL,
    [Remark]          NVARCHAR(200) NULL,
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 提成模板表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tBas_CommissionTemplate' AND xtype = 'U')
CREATE TABLE [tBas_CommissionTemplate] (
    [TemplateID]      VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateName]    NVARCHAR(100) NOT NULL DEFAULT '',
    [CalcMethod]      VARCHAR(20)  NOT NULL DEFAULT 'rate',
    [BaseAmount]      FLOAT        NOT NULL DEFAULT 0,
    [Rate]            FLOAT        NOT NULL DEFAULT 0,
    [Remark]          NVARCHAR(200) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 提成规则表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tBas_CommissionRule' AND xtype = 'U')
CREATE TABLE [tBas_CommissionRule] (
    [RuleID]          VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateID]      VARCHAR(30)  NOT NULL,
    [RuleType]        VARCHAR(20)  NOT NULL DEFAULT 'product',
    [RelID]           VARCHAR(30)  NOT NULL DEFAULT '',
    [Commission]      FLOAT        NOT NULL DEFAULT 0,
    [Remark]          NVARCHAR(200) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 定价模板表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tBas_PricingTemplate' AND xtype = 'U')
CREATE TABLE [tBas_PricingTemplate] (
    [TemplateID]      VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateName]    NVARCHAR(100) NOT NULL DEFAULT '',
    [PriceType]       VARCHAR(20)  NOT NULL DEFAULT 'sale',
    [CalcMethod]      VARCHAR(20)  NOT NULL DEFAULT 'rate',
    [Rate]            FLOAT        NOT NULL DEFAULT 0,
    [Remark]          NVARCHAR(200) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 定价规则表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tBas_PricingRule' AND xtype = 'U')
CREATE TABLE [tBas_PricingRule] (
    [RuleID]          VARCHAR(30)  NOT NULL PRIMARY KEY,
    [TemplateID]      VARCHAR(30)  NOT NULL,
    [RuleType]        VARCHAR(20)  NOT NULL DEFAULT 'product',
    [RelID]           VARCHAR(30)  NOT NULL DEFAULT '',
    [Multiplier]      FLOAT        NOT NULL DEFAULT 1,
    [Remark]          NVARCHAR(200) NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- 客户定价表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tBas_CustPrice' AND xtype = 'U')
CREATE TABLE [tBas_CustPrice] (
    [CustID]          VARCHAR(30)  NOT NULL,
    [GDSID]           VARCHAR(30)  NOT NULL,
    [Price]           FLOAT        NOT NULL DEFAULT 0,
    [PriceType]       VARCHAR(20)  NOT NULL DEFAULT 'custom',
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL,
    CONSTRAINT [PK_tBas_CustPrice] PRIMARY KEY ([CustID], [GDSID])
);

-- 通知表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_Notification' AND xtype = 'U')
CREATE TABLE [tSys_Notification] (
    [NotifyID]        VARCHAR(30)  NOT NULL PRIMARY KEY,
    [ToUser]          VARCHAR(20)  NOT NULL,
    [Title]           NVARCHAR(200) NOT NULL DEFAULT '',
    [Content]         NVARCHAR(500) NULL,
    [NotifyType]      VARCHAR(20)  NOT NULL DEFAULT 'system',
    [RelatedID]       VARCHAR(30)  NULL,
    [IsRead]          CHAR(1)      NOT NULL DEFAULT 'N',
    [ReadDate]        DATETIME     NULL,
    [CreateDate]      DATETIME     NULL
);

-- 备份记录表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_Backup' AND xtype = 'U')
CREATE TABLE [tSys_Backup] (
    [BackupID]        VARCHAR(30)  NOT NULL PRIMARY KEY,
    [BackupName]      NVARCHAR(100) NOT NULL DEFAULT '',
    [BackupType]      VARCHAR(20)  NOT NULL DEFAULT 'full',
    [BackupPath]      NVARCHAR(500) NULL,
    [BackupSize]      INT          NOT NULL DEFAULT 0,
    [BackupDate]      DATETIME     NULL,
    [BackupUser]      VARCHAR(20)  NULL,
    [State]           CHAR(1)      NOT NULL DEFAULT 'A'
);

-- 系统配置表
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tSys_Config' AND xtype = 'U')
CREATE TABLE [tSys_Config] (
    [ConfigKey]       VARCHAR(50)  NOT NULL PRIMARY KEY,
    [ConfigValue]     NVARCHAR(500) NULL,
    [Remark]          NVARCHAR(200) NULL,
    [EDate]           DATETIME     NULL,
    [EUser]           VARCHAR(20)  NULL
);

-- =============================================
-- 初始化数据
-- =============================================
IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'company_name')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('company_name', N'深华辉日化', N'公司名称', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'default_warehouse')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('default_warehouse', '', N'默认仓库ID', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'enable_stock_alert')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('enable_stock_alert', 'Y', N'启用库存预警', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'backup_enabled')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('backup_enabled', 'Y', N'启用自动备份', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'backup_interval')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('backup_interval', '24', N'备份间隔(小时)', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_Config WHERE ConfigKey = 'print_copies')
INSERT INTO tSys_Config (ConfigKey, ConfigValue, Remark, EDate, EUser) VALUES ('print_copies', '1', N'默认打印份数', GETDATE(), 'system');

-- 初始化打印模板（采购订单）
IF NOT EXISTS (SELECT 1 FROM tSys_PrintTemplate WHERE TemplateID = 'PT_PUR_ORDER')
INSERT INTO tSys_PrintTemplate (TemplateID, TemplateName, DocType, PaperWidth, PaperHeight, MarginTop, MarginBottom, MarginLeft, MarginRight, Orientation, Content, IsDefault, Remark, State, EDate, EUser)
VALUES ('PT_PUR_ORDER', N'采购订单默认模板', 'purchase_order', 210, 297, 10, 10, 10, 10, 'portrait', '{}', 'Y', N'系统默认采购订单打印模板', 'A', GETDATE(), 'system');

-- 初始化打印模板（销售订单）
IF NOT EXISTS (SELECT 1 FROM tSys_PrintTemplate WHERE TemplateID = 'PT_SAL_INV')
INSERT INTO tSys_PrintTemplate (TemplateID, TemplateName, DocType, PaperWidth, PaperHeight, MarginTop, MarginBottom, MarginLeft, MarginRight, Orientation, Content, IsDefault, Remark, State, EDate, EUser)
VALUES ('PT_SAL_INV', N'销售订单默认模板', 'sales_order', 210, 297, 10, 10, 10, 10, 'portrait', '{}', 'Y', N'系统默认销售订单打印模板', 'A', GETDATE(), 'system');

-- 初始化打印模板（调拨单）
IF NOT EXISTS (SELECT 1 FROM tSys_PrintTemplate WHERE TemplateID = 'PT_STK_MOVE')
INSERT INTO tSys_PrintTemplate (TemplateID, TemplateName, DocType, PaperWidth, PaperHeight, MarginTop, MarginBottom, MarginLeft, MarginRight, Orientation, Content, IsDefault, Remark, State, EDate, EUser)
VALUES ('PT_STK_MOVE', N'调拨单默认模板', 'transfer_order', 210, 297, 10, 10, 10, 10, 'portrait', '{}', 'Y', N'系统默认调拨单打印模板', 'A', GETDATE(), 'system');

-- 初始化打印模板（盘点单）
IF NOT EXISTS (SELECT 1 FROM tSys_PrintTemplate WHERE TemplateID = 'PT_STK_CHECK')
INSERT INTO tSys_PrintTemplate (TemplateID, TemplateName, DocType, PaperWidth, PaperHeight, MarginTop, MarginBottom, MarginLeft, MarginRight, Orientation, Content, IsDefault, Remark, State, EDate, EUser)
VALUES ('PT_STK_CHECK', N'盘点单默认模板', 'stock_check', 210, 297, 10, 10, 10, 10, 'portrait', '{}', 'Y', N'系统默认盘点单打印模板', 'A', GETDATE(), 'system');

-- 初始化打印模板（收据）
IF NOT EXISTS (SELECT 1 FROM tSys_PrintTemplate WHERE TemplateID = 'PT_RECEIPT')
INSERT INTO tSys_PrintTemplate (TemplateID, TemplateName, DocType, PaperWidth, PaperHeight, MarginTop, MarginBottom, MarginLeft, MarginRight, Orientation, Content, IsDefault, Remark, State, EDate, EUser)
VALUES ('PT_RECEIPT', N'收据默认模板', 'receipt', 210, 140, 5, 5, 5, 5, 'portrait', '{}', 'Y', N'系统默认收据打印模板', 'A', GETDATE(), 'system');

-- 初始化打印配置
IF NOT EXISTS (SELECT 1 FROM tSys_PrintConfig WHERE DocType = 'purchase_order')
INSERT INTO tSys_PrintConfig (DocType, TemplateID, Copies, AutoPrint, PrinterName, State, EDate, EUser) VALUES ('purchase_order', 'PT_PUR_ORDER', 1, 'N', '', 'A', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_PrintConfig WHERE DocType = 'sales_order')
INSERT INTO tSys_PrintConfig (DocType, TemplateID, Copies, AutoPrint, PrinterName, State, EDate, EUser) VALUES ('sales_order', 'PT_SAL_INV', 1, 'N', '', 'A', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_PrintConfig WHERE DocType = 'transfer_order')
INSERT INTO tSys_PrintConfig (DocType, TemplateID, Copies, AutoPrint, PrinterName, State, EDate, EUser) VALUES ('transfer_order', 'PT_STK_MOVE', 1, 'N', '', 'A', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_PrintConfig WHERE DocType = 'stock_check')
INSERT INTO tSys_PrintConfig (DocType, TemplateID, Copies, AutoPrint, PrinterName, State, EDate, EUser) VALUES ('stock_check', 'PT_STK_CHECK', 1, 'N', '', 'A', GETDATE(), 'system');

IF NOT EXISTS (SELECT 1 FROM tSys_PrintConfig WHERE DocType = 'receipt')
INSERT INTO tSys_PrintConfig (DocType, TemplateID, Copies, AutoPrint, PrinterName, State, EDate, EUser) VALUES ('receipt', 'PT_RECEIPT', 1, 'N', '', 'A', GETDATE(), 'system');

-- =============================================
-- 库存预占表（订单流程 P0 完善用）
-- 销售订单审核时预占库存，出库时校验+释放。
-- =============================================
IF NOT EXISTS (SELECT * FROM sysobjects WHERE name = 'tStk_Reserve' AND xtype = 'U')
CREATE TABLE [tStk_Reserve] (
    [ReserveID]       VARCHAR(30)   NOT NULL PRIMARY KEY,
    [DocType]         VARCHAR(20)   NOT NULL DEFAULT '',  -- sales_order / wholesale_order
    [DocID]           VARCHAR(30)   NOT NULL DEFAULT '',  -- 源单单据主键
    [DocNo]           VARCHAR(50)   NOT NULL DEFAULT '',  -- 源单单号
    [DetailID]        VARCHAR(30)   NOT NULL DEFAULT '',  -- 源单明细主键（按行释放）
    [GDSID]           VARCHAR(30)   NOT NULL DEFAULT '',
    [StkID]           VARCHAR(30)   NOT NULL DEFAULT '',
    [Qty]             FLOAT         NOT NULL DEFAULT 0,    -- 预占数量
    [ReleasedQty]     FLOAT         NOT NULL DEFAULT 0,    -- 已释放数量（出库核减时累加）
    [State]           CHAR(1)       NOT NULL DEFAULT 'A',  -- A=有效 X=已释放完
    [EDate]           DATETIME      NULL,
    [EUser]           VARCHAR(20)   NULL
);
-- 索引已由 add_docno_and_indexes.sql (IX_tStk_Reserve_DocType_DocID) 和 DB-02 (idx_Reserve_GDSID_StkID) 创建
-- 此处不再重复创建，避免重复索引（见 DB-02-cleanup-duplicate-indexes.sql）
