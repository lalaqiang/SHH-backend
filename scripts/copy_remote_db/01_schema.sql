-- DDL for tables missing in local
-- Generated: 2026-06-07 02:01:50
USE [TestERP];
GO
SET QUOTED_IDENTIFIER ON; SET ANSI_NULLS ON;
GO

IF OBJECT_ID('[dbo].[tPub_DocImg]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[tPub_DocImg] (
    [DocID] TGUID NOT NULL,
    [DocDID] TGUID NOT NULL,
    [ImgTypeID] TGUID NOT NULL,
    [DocImgID] TGUID NOT NULL,
    [ImgTitle] TNote,
    [Img] image,
    [ImgNote] TNote,
    [UpTime] datetime,
    [UpUser] TGUID
    ,CONSTRAINT [PK_tPub_DocImg] PRIMARY KEY CLUSTERED ([DocID], [DocDID], [ImgTypeID], [DocImgID])
);
END
GO

IF OBJECT_ID('[dbo].[tStk_StockHis]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[tStk_StockHis] (
    [GDSID] TGUID NOT NULL,
    [StkID] TGUID NOT NULL,
    [Kind] char(1) NOT NULL,
    [SortID] int IDENTITY(1,1) NOT NULL,
    [IOID] TGUID NOT NULL,
    [IONO] TCode NOT NULL,
    [IODate] datetime NOT NULL,
    [BTPID] TGUID NOT NULL,
    [IODetailID] TGUID NOT NULL,
    [EDate] datetime NOT NULL,
    [UN] smallint NOT NULL,
    [BefQty] TQty NOT NULL,
    [InQty] TQty NOT NULL,
    [OutQty] TQty NOT NULL,
    [AftQty] TQty NOT NULL,
    [Note] TNote
    ,CONSTRAINT [PK_tStk_StockHis] PRIMARY KEY CLUSTERED ([GDSID], [StkID], [Kind], [SortID])
);
END
GO

IF OBJECT_ID('[dbo].[tsys_GridInfo20201109New]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[tsys_GridInfo20201109New] (
    [GridID] uniqueidentifier NOT NULL,
    [GridCode] varchar(MAX),
    [GridDesc] nvarchar(MAX),
    [GetDataSQL] nvarchar(MAX),
    [KeyField] varchar(MAX),
    [MaxCount] int,
    [NoField] varchar(MAX),
    [NameField] varchar(MAX),
    [PKeyField] varchar(MAX),
    [DisPlayFields] varchar(MAX),
    [TableName] varchar(MAX)
);
END
GO

IF OBJECT_ID('[dbo].[tSys_TranHis]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[tSys_TranHis] (
    [TranID] TGUID NOT NULL,
    [DocID] TGUID NOT NULL,
    [RDate] datetime NOT NULL
    ,CONSTRAINT [PK_tSys_TranHis] PRIMARY KEY CLUSTERED ([TranID])
);
END
GO

IF OBJECT_ID('[dbo].[tSys_User]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[tSys_User] (
    [UserID] uniqueidentifier NOT NULL,
    [UserCode] nvarchar(MAX),
    [UserName] nvarchar(MAX),
    [RealName] nvarchar(MAX),
    [PassWordStr] nvarchar(MAX),
    [RuleID] uniqueidentifier,
    [EmpID] uniqueidentifier,
    [StkID] uniqueidentifier,
    [Phone] nvarchar(MAX),
    [Email] nvarchar(MAX),
    [Remark] nvarchar(MAX),
    [State] char(1),
    [Locked] int,
    [EDate] datetime,
    [EUser] nvarchar(MAX),
    [LUTime] datetime
    ,CONSTRAINT [PK_tSys_User] PRIMARY KEY CLUSTERED ([UserID])
);
END
GO

IF OBJECT_ID('[dbo].[表1]','U') IS NULL
BEGIN
CREATE TABLE [dbo].[表1] (
    [ID] bigint IDENTITY(1,1) NOT NULL,
    [FGC_Creator] nvarchar(MAX),
    [FGC_CreateDate] datetime,
    [FGC_LastModifier] nvarchar(MAX),
    [FGC_LastModifyDate] datetime,
    [FGC_Rowversion] timestamp NOT NULL,
    [FGC_UpdateHelp] nvarchar(MAX)
    ,CONSTRAINT [PK_表1] PRIMARY KEY CLUSTERED ([ID])
);
END
GO

