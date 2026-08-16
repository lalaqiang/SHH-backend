/* ============================================================================
   DB-04 单据号生成存储过程（SQL Server 2005 兼容）
   ----------------------------------------------------------------------------
   模块：DB-04
   目标：提供统一的并发安全单据号生成接口 sp_GetNextDocNo(@DocTypeID, @DocNo OUTPUT)
   包含：
     1) 建 tSys_DocNo / tSys_DocNoSeq 表（若不存在）
     2) 初始化 17 种单据类型配置（修正字段名：SoNo/SINo/IONo 等）
     3) 存储过程 sp_GetNextDocNo：UPDLOCK 串行化保证并发安全
   2005 兼容：
     - 不用 MERGE，用 IF EXISTS / UPDATE / INSERT
     - 不用 THROW，用 RAISERROR
     - 日期格式化用 CONVERT 不用 FORMAT
   幂等：可重复执行。
   ============================================================================ */

USE [TestERP];
GO
SET NOCOUNT ON;
GO

PRINT N'========================================';
PRINT N'DB-04 单据号生成存储过程安装开始';
PRINT N'时间：' + CONVERT(nvarchar(19), GETDATE(), 120);
PRINT N'========================================';
GO

/* ---------- 1. 建表 ---------- */
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = N'tSys_DocNo' AND xtype = N'U')
BEGIN
    CREATE TABLE [dbo].[tSys_DocNo] (
        DocTypeID    nvarchar(30)  NOT NULL,
        DocName      nvarchar(50)  NOT NULL DEFAULT N'',
        Prefix       nvarchar(20)  NOT NULL DEFAULT N'',
        TableName    nvarchar(60)  NOT NULL DEFAULT N'',
        FieldName    nvarchar(60)  NOT NULL DEFAULT N'',
        DateFormat   nvarchar(20)  NOT NULL DEFAULT N'YYYYMMDD',
        SeqPadding   int           NOT NULL DEFAULT 4,
        SeqStart     int           NOT NULL DEFAULT 1,
        DateReset    char(1)       NOT NULL DEFAULT N'Y',
        PeriodType   nvarchar(10)  NOT NULL DEFAULT N'DAY',
        State        char(1)       NOT NULL DEFAULT N'Y',
        Remark       nvarchar(200) NULL,
        LUTime       datetime      NULL DEFAULT GETDATE(),
        CONSTRAINT [PK_tSys_DocNo] PRIMARY KEY (DocTypeID)
    );
    PRINT N'[OK] 已建表 tSys_DocNo';
END
ELSE
    PRINT N'[SKIP] tSys_DocNo 已存在';
GO

IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = N'tSys_DocNoSeq' AND xtype = N'U')
BEGIN
    CREATE TABLE [dbo].[tSys_DocNoSeq] (
        DocTypeID    nvarchar(30)  NOT NULL,
        PeriodKey    nvarchar(20)  NOT NULL,
        CurrentSeq   bigint        NOT NULL DEFAULT 0,
        LUTime       datetime      NOT NULL DEFAULT GETDATE(),
        CONSTRAINT [PK_tSys_DocNoSeq] PRIMARY KEY (DocTypeID, PeriodKey)
    );
    PRINT N'[OK] 已建表 tSys_DocNoSeq';
END
ELSE
    PRINT N'[SKIP] tSys_DocNoSeq 已存在';
GO

/* ---------- 2. 初始化单据类型配置（修正字段名 + 删除引用不存在表的项）---------- */
-- 通用幂等插入辅助
IF OBJECT_ID(N'tmp_db04_ins', N'P') IS NOT NULL DROP PROC [dbo].[tmp_db04_ins];
GO
CREATE PROC [dbo].[tmp_db04_ins]
    @id nvarchar(30), @name nvarchar(50), @pre nvarchar(20),
    @tbl nvarchar(60), @fld nvarchar(60), @rmk nvarchar(200)
AS
BEGIN
    SET NOCOUNT ON;
    IF NOT EXISTS (SELECT 1 FROM [dbo].[tSys_DocNo] WHERE DocTypeID = @id)
    BEGIN
        INSERT INTO [dbo].[tSys_DocNo] (DocTypeID, DocName, Prefix, TableName, FieldName, Remark)
        VALUES (@id, @name, @pre, @tbl, @fld, @rmk);
        PRINT N'[OK] 配置 ' + @id + N' -> ' + @tbl + N'.' + @fld;
    END
    ELSE
        PRINT N'[SKIP] 配置 ' + @id + N' 已存在';
END
GO

-- 修正：tSal_Order 字段名是 SoNo（不是 OrderNo）；tSal_Inv 是 SINo；tStk_IO 是 IONo；
--       tStk_Move 是 MoveNO；tStk_Tran 是 TranNo
--       退货全部走 tStk_IO（无独立表）
EXEC tmp_db04_ins N'SO',  N'销售订单',    N'SO',  N'tSal_Order',         N'SoNo',   N'销售订单';
EXEC tmp_db04_ins N'SI',  N'销售出库单',  N'SD',  N'tStk_IO',            N'IONo',   N'销售出库 (tStk_IO Kind=SD)';
EXEC tmp_db04_ins N'SQ',  N'销售报价单',  N'SQ',  N'tSal_Quote',         N'QuoNo',  N'销售报价单';
EXEC tmp_db04_ins N'SR',  N'销售退货单',  N'SR',  N'tStk_IO',            N'IONo',   N'销售退货 (tStk_IO Kind=SR)';
EXEC tmp_db04_ins N'POS', N'POS销售',     N'POS', N'tStk_IO',            N'IONo',   N'POS销售 (tStk_IO Kind=POS)';
EXEC tmp_db04_ins N'PO',  N'采购订单',    N'PO',  N'tPur_Order',         N'PoNo',   N'采购订单';
EXEC tmp_db04_ins N'PQ',  N'采购报价单',  N'PQ',  N'tPur_Quote',         N'QuoNo',  N'采购报价单';
EXEC tmp_db04_ins N'RI',  N'采购入库单',  N'RI',  N'tStk_IO',            N'IONo',   N'采购入库 (tStk_IO Kind=RI)';
EXEC tmp_db04_ins N'TH',  N'采购退货单',  N'TH',  N'tStk_IO',            N'IONo',   N'采购退货 (tStk_IO Kind=TH)';
EXEC tmp_db04_ins N'MV',  N'调拨单',      N'MV',  N'tStk_Move',          N'MoveNO', N'库存调拨单';
EXEC tmp_db04_ins N'PD',  N'采购收货',    N'PD',  N'tStk_IO',            N'IONo',   N'采购收货 (tStk_IO Kind=PD)';
EXEC tmp_db04_ins N'OT',  N'其他出入库',  N'OT',  N'tStk_IO',            N'IONo',   N'零散出入库 (tStk_IO Kind=OT)';
EXEC tmp_db04_ins N'ZP',  N'门店直配',    N'ZP',  N'tStk_IO',            N'IONo',   N'门店直配 (tStk_IO Kind=ZP)';
EXEC tmp_db04_ins N'CHK', N'盘点单',      N'PD',  N'tStk_Tran',          N'TranNo', N'库存盘点单';
EXEC tmp_db04_ins N'PAY', N'付款单',      N'PAY', N'tAcc_PayOut',        N'PayOutNo', N'付款单';
EXEC tmp_db04_ins N'RCV', N'收款单',      N'RCV', N'tAcc_PayIn',         N'PayInNo', N'收款单';
EXEC tmp_db04_ins N'RP',  N'补货申请',    N'RP',  N'tStk_ReplenishApply', N'ReplenishApplyNo', N'补货申请单';
GO
DROP PROC [dbo].[tmp_db04_ins];
GO

/* ---------- 3. 存储过程 sp_GetNextDocNo ---------- */
IF OBJECT_ID(N'sp_GetNextDocNo', N'P') IS NOT NULL DROP PROCEDURE [dbo].[sp_GetNextDocNo];
GO
CREATE PROCEDURE [dbo].[sp_GetNextDocNo]
    @DocTypeID nvarchar(30),
    @DocNo     nvarchar(40) OUTPUT
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Prefix     nvarchar(20);
    DECLARE @DateFormat nvarchar(20);
    DECLARE @SeqPadding int;
    DECLARE @SeqStart   int;
    DECLARE @DateReset  char(1);
    DECLARE @PeriodType nvarchar(10);
    DECLARE @State      char(1);
    DECLARE @PeriodKey  nvarchar(20);
    DECLARE @DatePart   nvarchar(20);
    DECLARE @NextSeq    bigint;
    DECLARE @SeqStr     nvarchar(20);

    -- 取配置
    SELECT  @Prefix = Prefix, @DateFormat = DateFormat, @SeqPadding = SeqPadding,
            @SeqStart = SeqStart, @DateReset = DateReset, @PeriodType = PeriodType, @State = State
    FROM    [dbo].[tSys_DocNo]
    WHERE   DocTypeID = @DocTypeID;

    IF @@ROWCOUNT = 0
    BEGIN
        RAISERROR(N'单据类型 [%s] 未配置', 16, 1, @DocTypeID);
        RETURN -1;
    END
    IF @State <> N'Y'
    BEGIN
        RAISERROR(N'单据类型 [%s] 已停用', 16, 1, @DocTypeID);
        RETURN -2;
    END

    -- 计算 PeriodKey 和日期段（2005：用 CONVERT 不用 FORMAT）
    IF @PeriodType = N'DAY'
    BEGIN
        SET @DatePart = CONVERT(nvarchar(8), GETDATE(), 112);  -- YYYYMMDD
        SET @PeriodKey = @DatePart;
    END
    ELSE IF @PeriodType = N'MONTH'
    BEGIN
        SET @DatePart = CONVERT(nvarchar(6), GETDATE(), 112);  -- YYYYMM
        SET @PeriodKey = @DatePart;
    END
    ELSE
    BEGIN
        SET @DatePart = N'';
        SET @PeriodKey = N'FOREVER';
    END

    -- 按配置转换日期段格式
    IF @DateFormat = N'YYYYMMDD'
        SET @DatePart = CONVERT(nvarchar(8), GETDATE(), 112);
    ELSE IF @DateFormat = N'YYYYMM'
        SET @DatePart = CONVERT(nvarchar(6), GETDATE(), 112);
    ELSE IF @DateFormat = N'YYMMDD'
        SET @DatePart = RIGHT(CONVERT(nvarchar(8), GETDATE(), 112), 6);
    ELSE IF @DateFormat = N'YYMM'
        SET @DatePart = RIGHT(CONVERT(nvarchar(6), GETDATE(), 112), 4);
    ELSE
        SET @DatePart = N'';  -- NONE

    -- 并发安全取号：UPDLOCK + HOLDLOCK 串行化同一 (DocTypeID, PeriodKey)
    BEGIN TRAN;
    SELECT @NextSeq = CurrentSeq
      FROM [dbo].[tSys_DocNoSeq] WITH (UPDLOCK, HOLDLOCK)
     WHERE DocTypeID = @DocTypeID AND PeriodKey = @PeriodKey;

    IF @NextSeq IS NULL
    BEGIN
        SET @NextSeq = @SeqStart;
        INSERT INTO [dbo].[tSys_DocNoSeq] (DocTypeID, PeriodKey, CurrentSeq, LUTime)
        VALUES (@DocTypeID, @PeriodKey, @NextSeq, GETDATE());
    END
    ELSE
    BEGIN
        SET @NextSeq = @NextSeq + 1;
        UPDATE [dbo].[tSys_DocNoSeq]
           SET CurrentSeq = @NextSeq, LUTime = GETDATE()
         WHERE DocTypeID = @DocTypeID AND PeriodKey = @PeriodKey;
    END
    COMMIT TRAN;

    -- 序号补零（2005：用 RIGHT + REPLICATE）
    IF @SeqPadding > 0
        SET @SeqStr = RIGHT(REPLICATE(N'0', @SeqPadding) + CONVERT(nvarchar(20), @NextSeq), @SeqPadding);
    ELSE
        SET @SeqStr = CONVERT(nvarchar(20), @NextSeq);

    -- 拼单据号：Prefix + DatePart + SeqStr
    SET @DocNo = @Prefix + @DatePart + @SeqStr;

    RETURN 0;
END
GO

/* ---------- 4. 验证：试生成几个号 ---------- */
DECLARE @no nvarchar(40);
EXEC sp_GetNextDocNo N'SO', @no OUTPUT;  PRINT N'SO 示例: ' + @no;
EXEC sp_GetNextDocNo N'PO', @no OUTPUT;  PRINT N'PO 示例: ' + @no;
EXEC sp_GetNextDocNo N'MV', @no OUTPUT;  PRINT N'MV 示例: ' + @no;
EXEC sp_GetNextDocNo N'CHK', @no OUTPUT; PRINT N'CHK 示例: ' + @no;
GO

PRINT N'';
PRINT N'=== DB-04 完成 ===';
PRINT N'sp_GetNextDocNo 已就绪，并发安全（UPDLOCK+HOLDLOCK）。';
PRINT N'用法：DECLARE @no nvarchar(40); EXEC sp_GetNextDocNo N''SO'', @no OUTPUT;';
GO
