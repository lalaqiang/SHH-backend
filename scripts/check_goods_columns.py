# -*- coding: utf-8 -*-
"""查询 tBas_Goods 真实字段约束，作为前端必填项/校验依据。"""
import pymssql
import sys

try:
    conn = pymssql.connect(
        server='shenhuahui.f3322.org', port=1433,
        user='sa', password='sa123456', database='TestERP',
        timeout=60, login_timeout=30, charset='utf8'
    )
    cursor = conn.cursor()

    # 字段完整约束：是否可空、类型、长度、默认值、是否计算列/标识列
    cursor.execute("""
    SELECT
        c.COLUMN_NAME,
        c.ORDINAL_POSITION,
        c.DATA_TYPE,
        c.IS_NULLABLE,                       -- YES / NO
        c.CHARACTER_MAXIMUM_LENGTH,
        c.NUMERIC_PRECISION,
        c.NUMERIC_SCALE,
        c.COLUMN_DEFAULT,
        COLUMNPROPERTY(OBJECT_ID('tBas_Goods'), c.COLUMN_NAME, 'IsIdentity') AS IsIdentity,
        COLUMNPROPERTY(OBJECT_ID('tBas_Goods'), c.COLUMN_NAME, 'IsComputed') AS IsComputed
    FROM INFORMATION_SCHEMA.COLUMNS c
    WHERE c.TABLE_NAME = 'tBas_Goods'
    ORDER BY c.ORDINAL_POSITION
    """)
    rows = cursor.fetchall()

    print("=" * 90)
    print("tBas_Goods 字段约束 (共 %d 列)" % len(rows))
    print("=" * 90)
    header = "%-22s %-3s %-18s %-4s %-8s %-10s %-10s %-20s %-3s %-3s" % (
        "COLUMN", "#", "TYPE", "NULL", "LEN", "PREC", "SCALE", "DEFAULT", "ID", "CMP")
    print(header)
    print("-" * 90)
    for r in rows:
        (name, pos, dtype, nullable, charlen, prec, scale, default, is_id, is_cmp) = r
        print("%-22s %-3d %-18s %-4s %-8s %-10s %-10s %-20s %-3s %-3s" % (
            name, pos, dtype, nullable,
            str(charlen) if charlen else '-',
            str(prec) if prec else '-',
            str(scale) if scale else '-',
            (str(default) if default is not None else '-'),
            str(is_id), str(is_cmp)
        ))

    # NOT NULL 且无默认值的列 = 业务层真正必须提供值的字段
    print("\n" + "=" * 90)
    print("必须提供值的字段 (NOT NULL 且 无默认值 且 非计算列/非标识列):")
    print("=" * 90)
    cursor.execute("""
    SELECT c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH
    FROM INFORMATION_SCHEMA.COLUMNS c
    WHERE c.TABLE_NAME = 'tBas_Goods'
      AND c.IS_NULLABLE = 'NO'
      AND c.COLUMN_DEFAULT IS NULL
      AND COLUMNPROPERTY(OBJECT_ID('tBas_Goods'), c.COLUMN_NAME, 'IsComputed') = 0
      AND COLUMNPROPERTY(OBJECT_ID('tBas_Goods'), c.COLUMN_NAME, 'IsIdentity') = 0
    ORDER BY c.ORDINAL_POSITION
    """)
    for r in cursor.fetchall():
        print("  %-22s %-18s len=%s" % (r[0], r[1], r[2] or '-'))

    # 外键约束：哪些 ID 字段关联到哪张表
    print("\n" + "=" * 90)
    print("外键关系:")
    print("=" * 90)
    cursor.execute("""
    SELECT
        f.name AS constraint_name,
        COL_NAME(fc.parent_object_id, fc.parent_column_id) AS column_name,
        OBJECT_NAME(fc.referenced_object_id) AS ref_table,
        COL_NAME(fc.referenced_object_id, fc.referenced_column_id) AS ref_column
    FROM sys.foreign_keys f
    JOIN sys.foreign_key_columns fc ON f.object_id = fc.constraint_object_id
    WHERE f.parent_object_id = OBJECT_ID('tBas_Goods')
    ORDER BY COL_NAME(fc.parent_object_id, fc.parent_column_id)
    """)
    fk_rows = cursor.fetchall()
    if fk_rows:
        for r in fk_rows:
            print("  %-22s -> %-20s . %s" % (r[1], r[2], r[3]))
    else:
        print("  (无外键约束 — 数据层不强制引用完整性)")

    # 唯一约束 / 索引（如 GDSNO 是否唯一）
    print("\n" + "=" * 90)
    print("唯一索引 (可能影响 GDSNO 等编码唯一性):")
    print("=" * 90)
    cursor.execute("""
    SELECT i.name, i.is_unique, i.is_unique_constraint,
        STRING_AGG(COL_NAME(ic.object_id, ic.column_id), ', ') WITHIN GROUP (ORDER BY ic.key_ordinal) AS cols
    FROM sys.indexes i
    JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id
    WHERE i.object_id = OBJECT_ID('tBas_Goods') AND i.is_unique = 1
    GROUP BY i.name, i.is_unique, i.is_unique_constraint
    """)
    uniq_rows = cursor.fetchall()
    if uniq_rows:
        for r in uniq_rows:
            print("  %-30s unique=%s cols=[%s]" % (r[0], r[1], r[3]))
    else:
        print("  (无唯一索引)")

    conn.close()
except Exception as e:
    print("Error: %s" % e)
    sys.exit(1)
