-- ============================================================
-- 初始化缺失表脚本 (SQL Server 2005 兼容)
-- ============================================================

-- 1. tFin_Payment — 付款记录
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payment' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Payment (
        PaymentID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        PayNo       nvarchar(50)     NULL,
        PayDate     datetime         NULL,
        SuppID      uniqueidentifier NULL,
        DeptID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        StkID       uniqueidentifier NULL,
        Amount      decimal(18,2)    DEFAULT 0,
        PayMethod   varchar(20)      DEFAULT 'cash',
        BankName    nvarchar(100)    NULL,
        BankAccount nvarchar(50)     NULL,
        DocID       uniqueidentifier NULL,
        DocNo       nvarchar(50)     NULL,
        Note        nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'N',
        LUTime      datetime         DEFAULT GETDATE(),
        EUser       nvarchar(50)     NULL,
        EDate       datetime         DEFAULT GETDATE(),
        AUser       nvarchar(50)     NULL,
        ADate       datetime         NULL
    )
END
GO

-- 2. tFin_Receipt — 收款记录
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receipt' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Receipt (
        ReceiptID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        RcptNo      nvarchar(50)     NULL,
        RcptDate    datetime         NULL,
        CustID      uniqueidentifier NULL,
        DeptID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        StkID       uniqueidentifier NULL,
        Amount      decimal(18,2)    DEFAULT 0,
        PayMethod   varchar(20)      DEFAULT 'cash',
        BankName    nvarchar(100)    NULL,
        BankAccount nvarchar(50)     NULL,
        DocID       uniqueidentifier NULL,
        DocNo       nvarchar(50)     NULL,
        Note        nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'N',
        LUTime      datetime         DEFAULT GETDATE(),
        EUser       nvarchar(50)     NULL,
        EDate       datetime         DEFAULT GETDATE(),
        AUser       nvarchar(50)     NULL,
        ADate       datetime         NULL
    )
END
GO

-- 3. tFin_Payable — 应付账款
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Payable' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Payable (
        PayableID    uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        SuppID       uniqueidentifier NULL,
        DeptID       uniqueidentifier NULL,
        DocID        uniqueidentifier NULL,
        DocNo        nvarchar(50)     NULL,
        TotalAmt     decimal(18,2)    DEFAULT 0,
        PaidAmt      decimal(18,2)    DEFAULT 0,
        RemainAmt    decimal(18,2)    DEFAULT 0,
        PayableDate  datetime         NULL,
        DueDate      datetime         NULL,
        Status       varchar(20)      DEFAULT 'unpaid',
        Note         nvarchar(500)    NULL,
        State        char(1)          DEFAULT 'A',
        LUTime       datetime         DEFAULT GETDATE(),
        EUser        nvarchar(50)     NULL,
        EDate        datetime         DEFAULT GETDATE()
    )
END
GO

-- 4. tFin_Receivable — 应收账款
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_Receivable' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_Receivable (
        ReceivableID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        CustID         uniqueidentifier NULL,
        DeptID         uniqueidentifier NULL,
        DocID          uniqueidentifier NULL,
        DocNo          nvarchar(50)     NULL,
        TotalAmt       decimal(18,2)    DEFAULT 0,
        ReceivedAmt    decimal(18,2)    DEFAULT 0,
        RemainAmt      decimal(18,2)    DEFAULT 0,
        ReceivableDate datetime         NULL,
        DueDate        datetime         NULL,
        Status         varchar(20)      DEFAULT 'unpaid',
        Note           nvarchar(500)    NULL,
        State          char(1)          DEFAULT 'A',
        LUTime         datetime         DEFAULT GETDATE(),
        EUser          nvarchar(50)     NULL,
        EDate          datetime         DEFAULT GETDATE()
    )
END
GO

-- 5. tFin_CashFlow — 现金流
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tFin_CashFlow' AND xtype = 'U')
BEGIN
    CREATE TABLE tFin_CashFlow (
        CashFlowID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        FlowDate   datetime         NULL,
        FlowType   varchar(20)      NULL,
        Amount     decimal(18,2)    DEFAULT 0,
        SuppID     uniqueidentifier NULL,
        CustID     uniqueidentifier NULL,
        DeptID     uniqueidentifier NULL,
        EmpID      uniqueidentifier NULL,
        DocNo      nvarchar(50)     NULL,
        BankName   nvarchar(100)    NULL,
        Note       nvarchar(500)    NULL,
        State      char(1)          DEFAULT 'A',
        LUTime     datetime         DEFAULT GETDATE(),
        EUser      nvarchar(50)     NULL,
        EDate      datetime         DEFAULT GETDATE()
    )
END
GO

-- 6. tSys_RuleMenu — 角色菜单权限关联
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_RuleMenu' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_RuleMenu (
        RuleMenuID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        RuleID     uniqueidentifier NULL,
        MenuID     uniqueidentifier NULL,
        CanRead    int              DEFAULT 1,
        CanCreate  int              DEFAULT 0,
        CanUpdate  int              DEFAULT 0,
        CanDelete  int              DEFAULT 0,
        CanAudit   int              DEFAULT 0,
        CanPrint   int              DEFAULT 0,
        LUTime     datetime         DEFAULT GETDATE()
    )
END
GO

-- 7. tSys_UserRule — 用户角色关联
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_UserRule' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_UserRule (
        UserRuleID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        EmpID      uniqueidentifier NULL,
        RuleID     uniqueidentifier NULL,
        LUTime     datetime         DEFAULT GETDATE()
    )
END
GO

-- 8. tSys_TableColumnConfig — 表格列配置
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_TableColumnConfig' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_TableColumnConfig (
        ColumnConfigID uniqueidentifier   PRIMARY KEY DEFAULT NEWID(),
        EmpID          uniqueidentifier   NULL,
        TableName      nvarchar(100)      NULL,
        ConfigData     nvarchar(4000)     NULL,
        LUTime         datetime           DEFAULT GETDATE()
    )
END
GO

-- 9. tSys_UploadFile — 上传文件
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_UploadFile' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_UploadFile (
        FileID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        FileName nvarchar(200)    NULL,
        FilePath nvarchar(500)    NULL,
        FileSize int              DEFAULT 0,
        FileType nvarchar(50)     NULL,
        BizType  nvarchar(50)     NULL,
        BizID    uniqueidentifier NULL,
        State    char(1)          DEFAULT 'A',
        EUser    nvarchar(50)     NULL,
        EDate    datetime         DEFAULT GETDATE(),
        LUTime   datetime         DEFAULT GETDATE()
    )
END
GO

-- 10. tSys_User — 系统用户
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE name = 'tSys_User' AND xtype = 'U')
BEGIN
    CREATE TABLE tSys_User (
        UserID      uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        UserCode    nvarchar(50)     NULL,
        UserName    nvarchar(50)     NULL,
        RealName    nvarchar(50)     NULL,
        PassWordStr nvarchar(200)    NULL,
        RuleID      uniqueidentifier NULL,
        EmpID       uniqueidentifier NULL,
        StkID       uniqueidentifier NULL,
        Phone       nvarchar(20)     NULL,
        Email       nvarchar(100)    NULL,
        Remark      nvarchar(500)    NULL,
        State       char(1)          DEFAULT 'Y',
        Locked      int              DEFAULT 0,
        EDate       datetime         DEFAULT GETDATE(),
        EUser       nvarchar(50)     NULL,
        LUTime      datetime         DEFAULT GETDATE()
    )
END
GO
