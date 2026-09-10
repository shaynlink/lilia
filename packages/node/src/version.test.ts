import assert from "node:assert/strict";
import test from "node:test";
import { checkedVersion, checkedExpiry, readVersion, jsonNumbers } from "./version.js";
import { LiliaError } from "./types.js";

test("u64 validation rejects coercion and preserves both limits", () => {
  for (const value of [0n, 1n, 9007199254740993n, 18446744073709551615n]) {
    assert.equal(checkedVersion(value), value);
    assert.equal(readVersion(value.toString()), value);
  }
  for (const value of [-1n, 18446744073709551616n, 1, "1", null]) {
    assert.throws(() => checkedVersion(value as bigint), error => error instanceof LiliaError && error.code === "INVALID_INPUT" && !!error.requestId);
  }
  for (const value of [9007199254740992, "01", "+1", "", "18446744073709551616", -1n]) {
    assert.throws(() => readVersion(value));
  }
  assert.deepEqual(jsonNumbers({ nested: [4294967297n, -4294967297n] }), { nested: [4294967297, -4294967297] });
  assert.equal(checkedExpiry(4102444800000), 4102444800000);
  for (const expiry of [0.5, Number.NaN, Number.POSITIVE_INFINITY, 2 ** 53]) {
    assert.throws(() => checkedExpiry(expiry), LiliaError);
  }
});
