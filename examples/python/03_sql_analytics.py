"""03_sql_analytics.py - Declarative SQL analytics over JSON documents in LiliaDB.

Requirements:
- Run lilia-daemon with HTTP enabled:
    cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret
- Run this script:
    python3 examples/python/03_sql_analytics.py
"""

import os
import sys

from liliadb import LiliaClient

ENDPOINT = os.environ.get("LILIA_ENDPOINT", "http://127.0.0.1:4444")
TOKEN = os.environ.get("LILIA_TOKEN", "dev-secret")


def main() -> None:
    client = LiliaClient(endpoint=ENDPOINT, token=TOKEN)

    try:
        client.health()
    except Exception as exc:
        print(f"Error connecting to daemon at {ENDPOINT}: {exc}")
        print("\nPlease start the daemon with:")
        print("  cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret\n")
        sys.exit(1)

    space = "orders"

    print("\n--- 1. Seed Sample Orders ---")
    orders = [
        {"id": "ord_101", "customer": "Alice", "city": "Paris", "amount": 120.50, "status": "completed"},
        {"id": "ord_102", "customer": "Bob", "city": "Lyon", "amount": 45.00, "status": "completed"},
        {"id": "ord_103", "customer": "Charlie", "city": "Paris", "amount": 320.00, "status": "pending"},
        {"id": "ord_104", "customer": "Diana", "city": "Marseille", "amount": 89.90, "status": "completed"},
        {"id": "ord_105", "customer": "Eve", "city": "Lyon", "amount": 210.00, "status": "completed"},
    ]

    for ord_data in orders:
        client.json.put(space, ord_data["id"], ord_data)
    print(f"Seeded {len(orders)} orders into space '{space}'.")

    print("\n--- 2. Aggregation Query (Revenue by City) ---")
    query = """
        SELECT
            json_extract(value, '$.city') AS city,
            COUNT(*) AS order_count,
            ROUND(SUM(CAST(json_extract(value, '$.amount') AS REAL)), 2) AS total_revenue,
            ROUND(AVG(CAST(json_extract(value, '$.amount') AS REAL)), 2) AS avg_ticket
        FROM lilia_json
        WHERE space = 'orders' AND json_extract(value, '$.status') = 'completed'
        GROUP BY city
        ORDER BY total_revenue DESC
    """
    res = client.sql(query)
    print(f"Query returned {len(res)} rows. Columns: {res.columns}\n")

    # Access records as Python dictionaries
    for row in res.to_dicts():
        print(f"  City: {row['city']:<10} | Orders: {row['order_count']} | Revenue: {row['total_revenue']} € | Avg: {row['avg_ticket']} €")

    print("\n--- 3. Filter Query (Large Orders) ---")
    filter_query = """
        SELECT
            id,
            json_extract(value, '$.customer') AS customer,
            json_extract(value, '$.amount') AS amount,
            json_extract(value, '$.status') AS status
        FROM lilia_json
        WHERE space = 'orders' AND CAST(json_extract(value, '$.amount') AS REAL) > 100
        ORDER BY amount DESC
    """
    res_filtered = client.sql(filter_query)
    for order in res_filtered.to_dicts():
        print(f"  [{order['id']}] {order['customer']}: {order['amount']} € ({order['status']})")

    print("\n--- 4. Cleanup ---")
    for o in orders:
        client.json.delete(space, o["id"])
    print("Cleanup done.")


if __name__ == "__main__":
    main()
