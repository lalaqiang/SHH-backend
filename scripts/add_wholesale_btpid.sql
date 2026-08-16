-- ============================================================
-- 添加批发业务类型到 tBas_BillType
-- BTPID 与前端 client/src/config/enums.js 的 BTPID 枚举严格对齐
--   WHOLESALE        = 6D8E9880-30BC-41F0-A8DE-E27263453DE4  (批发出货)
--   WHOLESALE_RETURN = C174CC02-FB79-48D1-9D46-ECD5CEB6A8E9  (批发退货)
-- 用于区分批发和零售数据（共用 tSal_Order / tStk_IO / tSal_Quote 表）
-- ============================================================

USE [TestERP]
GO

-- 批发出货
IF NOT EXISTS (SELECT 1 FROM tBas_BillType WHERE BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4')
BEGIN
    INSERT INTO tBas_BillType (
        BTPID, BTPCode, BTPName, InOut, Kind, Flg,
        CodePreFix, CodeRule, MaxCode, Note, PYCode,
        State, LUTime, EUser, EDate, AUser, ADate, SUser, SDate,
        btpSD, GridID, WorkFlowID, DefSQL, ShareAll
    ) VALUES (
        '6D8E9880-30BC-41F0-A8DE-E27263453DE4',
        '10', N'批发出货', -1, N'WS', N'Sys',
        N'WO', NULL, NULL, NULL, N'PFCH',
        N'S', GETDATE(), NULL, GETDATE(), NULL, NULL, NULL, NULL,
        N'1', NULL, NULL, NULL, N'Y'
    )
END
GO

-- 批发退货
IF NOT EXISTS (SELECT 1 FROM tBas_BillType WHERE BTPID = 'C174CC02-FB79-48D1-9D46-ECD5CEB6A8E9')
BEGIN
    INSERT INTO tBas_BillType (
        BTPID, BTPCode, BTPName, InOut, Kind, Flg,
        CodePreFix, CodeRule, MaxCode, Note, PYCode,
        State, LUTime, EUser, EDate, AUser, ADate, SUser, SDate,
        btpSD, GridID, WorkFlowID, DefSQL, ShareAll
    ) VALUES (
        'C174CC02-FB79-48D1-9D46-ECD5CEB6A8E9',
        '11', N'批发退货', 1, N'WSR', N'Sys',
        N'WR', NULL, NULL, NULL, N'PFTH',
        N'S', GETDATE(), NULL, GETDATE(), NULL, NULL, NULL, NULL,
        N'1', NULL, NULL, NULL, N'Y'
    )
END
GO

-- ============================================================
-- 更新已有的批发订单 BTPID（根据单号前缀 WO 识别）
-- 旧值 00000000-0000-0000-0000-000000000010 统一迁移到新 UUID
-- ============================================================
UPDATE tSal_Order SET BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4'
WHERE SoNo LIKE 'WO%'
  AND ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') IN (
      '00000000-0000-0000-0000-000000000000',
      '00000000-0000-0000-0000-000000000010'
  )
GO

-- 更新已有的批发出库 BTPID（根据单号前缀 WI + Kind=SD 识别）
UPDATE tStk_IO SET BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4'
WHERE Kind = 'SD' AND IONo LIKE 'WI%'
  AND ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') IN (
      '00000000-0000-0000-0000-000000000000',
      '00000000-0000-0000-0000-000000000010'
  )
GO

-- 更新已有的批发退货 BTPID
UPDATE tStk_IO SET BTPID = 'C174CC02-FB79-48D1-9D46-ECD5CEB6A8E9'
WHERE Kind = 'SR' AND IONo LIKE 'WR%'
  AND ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') IN (
      '00000000-0000-0000-0000-000000000000',
      '00000000-0000-0000-0000-000000000010'
  )
GO

-- 更新已有的批发报价 BTPID
UPDATE tSal_Quote SET BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4'
WHERE SQNo LIKE 'WQ%'
  AND ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') IN (
      '00000000-0000-0000-0000-000000000000',
      '00000000-0000-0000-0000-000000000010'
  )
GO

-- 更新已有的批发调价 BTPID
UPDATE tSal_AdjPrice SET BTPID = '6D8E9880-30BC-41F0-A8DE-E27263453DE4'
WHERE SAPNo LIKE 'SAP%'
  AND ISNULL(BTPID, '00000000-0000-0000-0000-000000000000') IN (
      '00000000-0000-0000-0000-000000000000',
      '00000000-0000-0000-0000-000000000010'
  )
GO
