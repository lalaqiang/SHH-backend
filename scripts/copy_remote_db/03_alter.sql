-- ALTER statements to add missing columns to local
USE [TestERP];
GO

IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tBas_Emp]') AND name = 'SuppID')
    ALTER TABLE [dbo].[tBas_Emp] ADD [SuppID] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tBas_Emp]') AND name = 'CustID')
    ALTER TABLE [dbo].[tBas_Emp] ADD [CustID] uniqueidentifier NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tBas_Goods]') AND name = 'GDSPropertyName')
    ALTER TABLE [dbo].[tBas_Goods] ADD [GDSPropertyName] nvarchar(50) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tBas_Goods]') AND name = 'GDSAbc')
    ALTER TABLE [dbo].[tBas_Goods] ADD [GDSAbc] varchar(5) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Note1')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Note1] varchar(50) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Qty3')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Qty3] decimal(18,2) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Note4')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Note4] nvarchar(250) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Note2')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Note2] varchar(50) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Note3')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Note3] varchar(50) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tPub_TmpStkQty]') AND name = 'Qty2')
    ALTER TABLE [dbo].[tPub_TmpStkQty] ADD [Qty2] decimal(18,2) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tSys_RegMD]') AND name = 'Flg')
    ALTER TABLE [dbo].[tSys_RegMD] ADD [Flg] varchar(100) NULL;
GO
IF NOT EXISTS (SELECT * FROM sys.columns WHERE object_id = OBJECT_ID('[dbo].[tSys_RegMD]') AND name = 'MDcallname')
    ALTER TABLE [dbo].[tSys_RegMD] ADD [MDcallname] varchar(100) NULL;
GO
