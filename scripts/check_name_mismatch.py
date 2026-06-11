import pymssql, sys

try:
    conn = pymssql.connect(server='shenhuahui.f3322.org', port=1433, user='sa', password='sa123456', database='TestERP', timeout=60, login_timeout=30)
    cursor = conn.cursor()

    # Compare tBas_Goods.BrandName (table's own) vs tBas_Brand.BrandName (joined)
    cursor.execute("""
    SELECT TOP 10 
        g.GDSNO, g.GDSDesc,
        g.BrandName AS GoodsBrandName,
        b.BrandName AS JoinedBrandName,
        CASE WHEN g.BrandName = b.BrandName OR (g.BrandName IS NULL AND b.BrandName IS NULL) THEN 'MATCH' ELSE 'MISMATCH' END AS BrandMatch,
        g.GDSTypeName AS GoodsTypeName,
        gt.GDSTypeName AS JoinedTypeName,
        CASE WHEN g.GDSTypeName = gt.GDSTypeName OR (g.GDSTypeName IS NULL AND gt.GDSTypeName IS NULL) THEN 'MATCH' ELSE 'MISMATCH' END AS TypeMatch,
        g.GDSKindName AS GoodsKindName,
        gk.GDSKindName AS JoinedKindName,
        g.DeaTypeName AS GoodsDeaName,
        dt.DeaTypeName AS JoinedDeaName,
        g.SuppName AS GoodsSuppName,
        s.SuppName AS JoinedSuppName
    FROM tBas_Goods g
    LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
    LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
    LEFT JOIN tBas_GDSKind gk ON g.GDSKindID = gk.GDSKindID
    LEFT JOIN tBas_DeaType dt ON g.DeaTypeID = dt.DeaTypeID
    LEFT JOIN tBas_Supp s ON g.SuppID = s.SuppID
    WHERE g.State <> 'D'
    """)
    rows = cursor.fetchall()
    print("GDSNO | GoodsBrand | JoinedBrand | Match | GoodsType | JoinedType | Match")
    for row in rows:
        gno = str(row[0])[:15]
        gb = str(row[2])[:15] if row[2] else 'NULL'
        jb = str(row[3])[:15] if row[3] else 'NULL'
        bm = row[4]
        gt = str(row[5])[:15] if row[5] else 'NULL'
        jt = str(row[6])[:15] if row[6] else 'NULL'
        tm = row[7]
        print(f"  {gno} | {gb} | {jb} | {bm} | {gt} | {jt} | {tm}")

    # Count mismatches
    cursor.execute("""
    SELECT 
        SUM(CASE WHEN g.BrandName <> b.BrandName OR (g.BrandName IS NOT NULL AND b.BrandName IS NULL) OR (g.BrandName IS NULL AND b.BrandName IS NOT NULL) THEN 1 ELSE 0 END) as BrandMismatch,
        SUM(CASE WHEN g.GDSTypeName <> gt.GDSTypeName OR (g.GDSTypeName IS NOT NULL AND gt.GDSTypeName IS NULL) OR (g.GDSTypeName IS NULL AND gt.GDSTypeName IS NOT NULL) THEN 1 ELSE 0 END) as TypeMismatch,
        SUM(CASE WHEN g.SuppName <> s.SuppName OR (g.SuppName IS NOT NULL AND s.SuppName IS NULL) OR (g.SuppName IS NULL AND s.SuppName IS NOT NULL) THEN 1 ELSE 0 END) as SuppMismatch,
        COUNT(*) as Total
    FROM tBas_Goods g
    LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
    LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
    LEFT JOIN tBas_Supp s ON g.SuppID = s.SuppID
    WHERE g.State <> 'D'
    """)
    row = cursor.fetchone()
    print(f"\nTotal: {row[3]}, BrandMismatch: {row[0]}, TypeMismatch: {row[1]}, SuppMismatch: {row[2]}")

    conn.close()
except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)
