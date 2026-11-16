// The access dialog's lookup-then-approve: the code approved is exactly the
// code looked up, whatever happens to the input meanwhile, and a late
// answer to an older lookup never becomes the one approved.
import test from "node:test";
import assert from "node:assert/strict";
import { devicePairing } from "../src/dashboard/web/operators-admin.mjs";

function server() {
  const calls = [];
  const waiting = [];
  const request = (path, init) => {
    const body = JSON.parse(init.body);
    calls.push({ path, body });
    if (path === "/api/devices/lookup") {
      return new Promise((resolve) => waiting.push(() => resolve({ request: { name: `device ${body.user_code}`, scope: "operate" } })));
    }
    return Promise.resolve({ approved: { name: "x", scope: body.scope } });
  };
  return { calls, request, answer: () => waiting.shift()() };
}

test("approve sends the code that was looked up, not what the field holds later", async () => {
  const s = server();
  const flow = devicePairing(s.request);
  const field = { value: "AAAA-BBBB" };
  const looking = flow.lookup(field.value);
  field.value = "CCCC-DDDD"; // edited while the lookup is in flight
  s.answer();
  assert.equal((await looking).name, "device AAAA-BBBB");
  await flow.approve("operate");
  assert.deepEqual(s.calls.at(-1), { path: "/api/devices/approve", body: { user_code: "AAAA-BBBB", scope: "operate" } });
});

test("a late answer to an older lookup is void", async () => {
  const s = server();
  const flow = devicePairing(s.request);
  const first = flow.lookup("AAAA-BBBB");
  const second = flow.lookup("CCCC-DDDD");
  s.answer();
  s.answer();
  assert.equal(await first, null);
  assert.equal((await second).name, "device CCCC-DDDD");
  await flow.approve("decide");
  assert.equal(s.calls.at(-1).body.user_code, "CCCC-DDDD");
});

test("nothing is approved without a lookup, after a cancel, or twice", async () => {
  const s = server();
  const flow = devicePairing(s.request);
  await assert.rejects(flow.approve("operate"));
  const looking = flow.lookup("AAAA-BBBB");
  s.answer();
  await looking;
  flow.cancel();
  await assert.rejects(flow.approve("operate"));
  const again = flow.lookup("AAAA-BBBB");
  s.answer();
  await again;
  await flow.approve("operate");
  await assert.rejects(flow.approve("operate"), "one approval per lookup");
  assert.equal(s.calls.filter((c) => c.path === "/api/devices/approve").length, 1);
});
