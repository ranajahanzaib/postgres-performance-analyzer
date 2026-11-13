# Safety

Normal diagnostics read PostgreSQL statistics and catalogs. They do not install extensions, change settings, reset statistics, create indexes, or execute captured query text. The EXPLAIN command defaults to nonexecuting `EXPLAIN (FORMAT JSON)`. EXPLAIN ANALYZE runs the target SQL and requires both `--analyze` and `--allow-execution`.

PostgreSQL connections to non-loopback TCP hosts require TLS and use the platform's trusted certificate store. A remote URL explicitly setting `sslmode=disable` is refused. Loopback and Unix-socket connections default to plaintext unless the URL explicitly requests TLS; this keeps the disposable local demo simple. A custom database CA must be installed in the platform trust store before connecting.

Treat explicit SQL, plans, predicates, connection strings, and artifacts as sensitive. SQL is excluded from query artifacts unless `--include-sql` is set. Credentials are obtained from `PGPA_DATABASE_URL` or `DATABASE_URL`, not printed by the program. Give the role only needed visibility; `pg_read_all_stats` expands visibility to other sessions and should be granted deliberately.

The Compose demo binds only to loopback and uses fixed credentials intended solely for its disposable local database. These values are public demonstration credentials, not secrets; do not reuse them. Demo initialization creates roles, tables, and data in the Compose database. Do not point it at any database containing data you need to preserve.
