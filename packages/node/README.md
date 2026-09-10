# @liliadb/node

```ts
import { Database } from "@liliadb/node";

const db = await Database.open({ path: "app.lilia" });
await db.json.put("users", "ada", { name: "Ada" });
console.log(await db.json.get("users", "ada"));
await db.close();
```

## Version precision

Use `bigint` for conditional writes, for example `{ ifVersion: 9007199254740993n }`.
Versions returned by reads, scans and mutations remain `bigint`; never convert them
to `number` for a subsequent write. Invalid or out-of-u64 expectations fail with
`INVALID_INPUT`. The current SQLite format caps stored versions at `i64::MAX`;
attempting to increment beyond that limit fails atomically with `STORAGE`.

CLI/MCP JSON versions and conflict details are decimal strings, not JSON numbers.
The SDK and native addon should be upgraded together. JSON document numbers keep
their normal JavaScript semantics, independently of version precision.
