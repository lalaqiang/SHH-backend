import pymssql
import sys

try:
    conn = pymssql.connect(server='shenhuahui.f3322.org', port=1433, user='sa', password='sa123456', database='TestERP', timeout=60, login_timeout=30)
    cursor = conn.cursor()

    # Check if BrandID in tBas_Goods actually exists in tBas_Brand
    cursor.execute("""
    SELECT TOP 10 g.GDSNO, g.GDSDesc, 
        g.GDSTypeID, gt.GDSTypeName,
        g.BrandID, 
        (SELECT COUNT(*) FROM tBas_Brand b WHERE b.BrandID = g.BrandID) as BrandExists,
        (SELECT COUNT(*) FROM tBas_GDSType t2 WHERE t2.GDSTypeID = g.BrandID) as BrandID_IsGDSType,
        g.GDSKindID,
        (SELECT COUNT(*) FROM tBas_GDSKind k WHERE k.GDSKindID = g.GDSKindID) as KindExists,
        g.DeaTypeID,
        (SELECT COUNT(*) FROM tBas_DeaType d WHERE d.DeaTypeID = g.DeaTypeID) as DeaExists,
        g.SuppID,
        (SELECT COUNT(*) FROM tBas_Supp s WHERE s.SuppID = g.SuppID) as SuppExists
    FROM tBas_Goods g
    LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
    WHERE g.State <> 'D'
    """)
    rows = cursor.fetchall()
    cols = [desc[0] for desc in cursor.description]
    print(" | ".join(cols))
    for row in rows:
        vals = [str(v)[:30] if v is not None else 'NULL' for v in row]
        print(" | ".join(vals))

    # Count how many BrandIDs don't exist in tBas_Brand
    cursor.execute("""
    SELECT 
        (SELECT COUNT(*) FROM tBas_Goods g WHERE g.State <> 'D' AND g.BrandID IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tBas_Brand b WHERE b.BrandID = g.BrandID)) as BadBrandCount,
        (SELECT COUNT(*) FROM tBas_Goods g WHERE g.State <> 'D' AND g.BrandID IS NOT NULL AND EXISTS (SELECT 1 FROM tBas_Brand b WHERE b.BrandID = g.BrandID)) as GoodBrandCount,
        (SELECT COUNT(*) FROM tBas_Goods g WHERE g.State <> 'D' AND g.BrandID IS NOT NULL AND EXISTS (SELECT 1 FROM tBas_GDSType t2 WHERE t2.GDSTypeID = g.BrandID)) as BrandID_IsGDSType,
        (SELECT COUNT(*) FROM tBas_Goods) as TotalGoods
    """)
    row = cursor.fetchone()
    print(f"\nBad BrandID (not in tBas_Brand): {row[0]}")
    print(f"Good BrandID (in tBas_Brand): {row[1]}")
    print(f"BrandID is actually a GDSTypeID: {row[2]}")
    print(f"Total goods: {row[3]}")

    # Check if any ALTER TABLE was done that might have swapped columns
    cursor.execute("""
    SELECT COLUMN_NAME, ORDINAL_POSITION, DATA_TYPE 
    FROM INFORMATION_SCHEMA.COLUMNS 
    WHERE TABLE_NAME = 'tBas_Goods' 
    ORDER BY ORDINAL_POSITION
    """)
    rows = cursor.fetchall()
    print("\n=== tBas_Goods column order ===")
    for row in rows:
        print(f"  {row[1]:3d} | {row[0]:30s} | {row[2]}")

    conn.close()
except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)
