# @liliadb/node

```ts
import { Database } from "@liliadb/node";

const db = await Database.open({ path: "app.lilia" });
await db.json.put("users", "ada", { name: "Ada" });
console.log(await db.json.get("users", "ada"));
await db.close();
```
