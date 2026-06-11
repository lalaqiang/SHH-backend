IF NOT EXISTS (SELECT * FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = 'tSys_OperLog')
BEGIN
    CREATE TABLE [tSys_OperLog] (
        [LogID] UNIQUEIDENTIFIER DEFAULT NEWID() PRIMARY KEY,
        [Module] NVARCHAR(50) NOT NULL,
        [RecordID] NVARCHAR(100) NULL,
        [OperationType] NVARCHAR(20) NOT NULL,
        [Content] NVARCHAR(500) NULL,
        [BeforeData] NVARCHAR(MAX) NULL,
        [AfterData] NVARCHAR(MAX) NULL,
        [OperatorID] NVARCHAR(100) NULL,
        [OperatorName] NVARCHAR(50) NULL,
        [OperTime] DATETIME DEFAULT GETDATE(),
        [IPAddress] NVARCHAR(50) NULL
    )
END
