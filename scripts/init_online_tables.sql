-- ============================================================
-- 在线商城模块 - 初始化表脚本
-- 兼容 SQL Server 2005
-- ============================================================

-- 1. tOnline_Goods - 在线商品池
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE xtype='U' AND name='tOnline_Goods')
BEGIN
    CREATE TABLE tOnline_Goods (
        OnlineGDSID    uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        GDSID          uniqueidentifier NULL,
        SaleType       varchar(20)      NOT NULL DEFAULT 'normal',
        ClearancePrice decimal(18,2)    NOT NULL DEFAULT 0,
        MaxOrderQty    int              NOT NULL DEFAULT 0,
        Sort           int              NOT NULL DEFAULT 0,
        Status         int              NOT NULL DEFAULT 1,
        StkID          uniqueidentifier NULL,
        ImageUrl       nvarchar(500)    NOT NULL DEFAULT '',
        State          char(1)          NOT NULL DEFAULT 'A',
        EUser          nvarchar(50)     NULL,
        EDate          datetime         NOT NULL DEFAULT GETDATE(),
        LUTime         datetime         NOT NULL DEFAULT GETDATE()
    )
END
GO

-- 2. tOnline_Order - 在线订单主表
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE xtype='U' AND name='tOnline_Order')
BEGIN
    CREATE TABLE tOnline_Order (
        OnlineOrderID   uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        OrderNo         nvarchar(50)     NOT NULL DEFAULT '',
        EmpID           uniqueidentifier NULL,
        EmpName         nvarchar(50)     NULL,
        ContactName     nvarchar(50)     NULL,
        ContactPhone    nvarchar(20)     NULL,
        Address         nvarchar(200)    NULL,
        TotalAmt        decimal(18,2)    NOT NULL DEFAULT 0,
        PaymentMethod   varchar(20)      NOT NULL DEFAULT 'cod',
        PaymentStatus   varchar(20)      NOT NULL DEFAULT 'unpaid',
        PaymentTradeNo  nvarchar(100)    NULL,
        PaymentProof    nvarchar(500)    NULL,
        DeliveryType    varchar(20)      NOT NULL DEFAULT 'express',
        Status          varchar(20)      NOT NULL DEFAULT 'pending',
        SalesDocNo      nvarchar(50)     NULL,
        OperatorID      uniqueidentifier NULL,
        OperatorName    nvarchar(50)     NULL,
        ConfirmTime     datetime         NULL,
        TrackingNo      nvarchar(100)    NULL,
        TrackingCompany nvarchar(50)     NULL,
        ShipStatus      varchar(20)      NOT NULL DEFAULT 'pending',
        ShipTime        datetime         NULL,
        Remark          nvarchar(500)    NULL,
        State           char(1)          NOT NULL DEFAULT 'A',
        EUser           nvarchar(50)     NULL,
        EDate           datetime         NOT NULL DEFAULT GETDATE(),
        LUTime          datetime         NOT NULL DEFAULT GETDATE()
    )

    CREATE UNIQUE NONCLUSTERED INDEX IX_tOnline_Order_OrderNo
        ON tOnline_Order(OrderNo)
END
GO

-- 3. tOnline_OrderDetail - 在线订单明细
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE xtype='U' AND name='tOnline_OrderDetail')
BEGIN
    CREATE TABLE tOnline_OrderDetail (
        OnlineOrderDtlID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        OnlineOrderID    uniqueidentifier NULL,
        GDSID            uniqueidentifier NULL,
        GDSNO            nvarchar(50)     NULL,
        GDSDesc          nvarchar(200)    NULL,
        Qty              int              NOT NULL DEFAULT 0,
        Price            decimal(18,2)    NOT NULL DEFAULT 0,
        Amt              decimal(18,2)    NOT NULL DEFAULT 0,
        SaleType         varchar(20)      NOT NULL DEFAULT 'normal',
        CostPrice        decimal(18,2)    NOT NULL DEFAULT 0,
        State            char(1)          NOT NULL DEFAULT 'A',
        LUTime           datetime         NOT NULL DEFAULT GETDATE()
    )

    CREATE NONCLUSTERED INDEX IX_tOnline_OrderDetail_OnlineOrderID
        ON tOnline_OrderDetail(OnlineOrderID)
END
GO

-- 4. tOnline_Address - 收货地址簿
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE xtype='U' AND name='tOnline_Address')
BEGIN
    CREATE TABLE tOnline_Address (
        AddressID    uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        EmpID        uniqueidentifier NULL,
        ContactName  nvarchar(50)     NULL,
        Phone        nvarchar(20)     NULL,
        Province     nvarchar(50)     NULL,
        City         nvarchar(50)     NULL,
        District     nvarchar(50)     NULL,
        Address      nvarchar(200)    NULL,
        IsDefault    int              NOT NULL DEFAULT 0,
        State        char(1)          NOT NULL DEFAULT 'A',
        EUser        nvarchar(50)     NULL,
        EDate        datetime         NOT NULL DEFAULT GETDATE(),
        LUTime       datetime         NOT NULL DEFAULT GETDATE()
    )

    CREATE NONCLUSTERED INDEX IX_tOnline_Address_EmpID
        ON tOnline_Address(EmpID)
END
GO

-- 5. tOnline_PaymentConfig - 支付配置
IF NOT EXISTS (SELECT 1 FROM sysobjects WHERE xtype='U' AND name='tOnline_PaymentConfig')
BEGIN
    CREATE TABLE tOnline_PaymentConfig (
        PaymentConfigID uniqueidentifier PRIMARY KEY DEFAULT NEWID(),
        PCode           varchar(20)      NOT NULL DEFAULT '',
        PName           nvarchar(50)     NULL,
        PKind           nvarchar(20)     NOT NULL DEFAULT 'payment',
        PValue          nvarchar(500)    NULL,
        PHelp           nvarchar(500)    NULL,
        QRCodeUrl       nvarchar(500)    NULL,
        IsPersonal      int              NOT NULL DEFAULT 0,
        Enabled         int              NOT NULL DEFAULT 1,
        Sort            int              NOT NULL DEFAULT 0,
        State           char(1)          NOT NULL DEFAULT 'A',
        EUser           nvarchar(50)     NULL,
        EDate           datetime         NOT NULL DEFAULT GETDATE(),
        LUTime          datetime         NOT NULL DEFAULT GETDATE()
    )

    CREATE UNIQUE NONCLUSTERED INDEX IX_tOnline_PaymentConfig_PCode
        ON tOnline_PaymentConfig(PCode)
END
GO

-- 默认支付配置数据
IF NOT EXISTS (SELECT 1 FROM tOnline_PaymentConfig WHERE PCode = 'cod')
    INSERT INTO tOnline_PaymentConfig (PaymentConfigID, PCode, PName, PKind, PHelp, IsPersonal, Enabled, Sort)
    VALUES (NEWID(), 'cod', N'货到付款', 'payment', N'货到付款，无需在线支付', 0, 1, 1)

IF NOT EXISTS (SELECT 1 FROM tOnline_PaymentConfig WHERE PCode = 'alipay')
    INSERT INTO tOnline_PaymentConfig (PaymentConfigID, PCode, PName, PKind, PHelp, IsPersonal, Enabled, Sort)
    VALUES (NEWID(), 'alipay', N'支付宝', 'payment', N'支付宝在线支付', 0, 0, 2)

IF NOT EXISTS (SELECT 1 FROM tOnline_PaymentConfig WHERE PCode = 'wechat')
    INSERT INTO tOnline_PaymentConfig (PaymentConfigID, PCode, PName, PKind, PHelp, IsPersonal, Enabled, Sort)
    VALUES (NEWID(), 'wechat', N'微信支付', 'payment', N'微信在线支付', 0, 0, 3)

IF NOT EXISTS (SELECT 1 FROM tOnline_PaymentConfig WHERE PCode = 'bank')
    INSERT INTO tOnline_PaymentConfig (PaymentConfigID, PCode, PName, PKind, PHelp, IsPersonal, Enabled, Sort)
    VALUES (NEWID(), 'bank', N'银行转账', 'payment', N'银行转账，需上传付款凭证', 0, 0, 4)
GO
