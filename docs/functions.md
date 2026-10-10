# Functions

A function is a small program behind your API. Porchly runs it when someone calls
`/api/fn/<name>`, and whatever it returns goes back to the caller. Use them to send an email
when an order comes in, accept a webhook from Stripe or GitHub, or combine several tables
into one answer.

Functions are written in **JavaScript or TypeScript** and run with [Bun](https://bun.sh): the real
language with `fetch`, timers, crypto, Node built-ins and npm packages. Porchly doesn't control
or watch what a function does. Only add code from people you trust.

## Turning them on
Functions are off until you want them. In the app, click **Functions** in the sidebar and then
**Turn on Functions**. Porchly downloads Bun once (about 40 MB). It's the official release, checked
against a checksum built into Porchly, and kept in `tools/` next to your project. Older x64
computers get Bun's "baseline" build automatically.

From the command line:
```sh
porchly-cli run --project my.db --cmd functions.enable
```
To use a Bun you already have instead, set `PORCHLY_BUN=/path/to/bun`.

Turning Functions off keeps their code. `functions.disable {"remove_runtime": true}` also deletes the
downloaded Bun.

## Where the code lives
```
my-project/
  porchly.db
  functions/
    send_invoice/
      main.ts          ← the code (main.js works too)
      function.json    ← Porchly's settings: who can run it, variables
  logs/functions/
    send_invoice.log   ← what the function printed
  tools/
    bun-1.4.2/bun      ← downloaded once
```
They're ordinary files, so you can edit them in the app or in any editor. Saving is enough; the
next call runs the new code. Deleted functions are moved to `functions/.deleted/`.

## Writing one
A function exports an object with a `fetch` method. It receives a standard web
[`Request`](https://developer.mozilla.org/en-US/docs/Web/API/Request) and returns a
[`Response`](https://developer.mozilla.org/en-US/docs/Web/API/Response). This is the same shape
Cloudflare Workers, Bun and Deno use, so code can move between them.

```ts
export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    return Response.json({ hello: url.searchParams.get("name") ?? "world" });
  },
};
```

- **Any method**, headers, status codes, binary bodies and streaming work.
- **Paths below the function's address reach it too.** `/api/fn/shop/orders/7?x=1` runs `shop`
  with `new URL(request.url).pathname == "/orders/7"`, so one function can be a small API.
- **Who called:** the caller's key never reaches the function. Instead, Porchly adds
  `X-Porchly-Key-Name` (the key's name) and `X-Porchly-Key-Write` (`true` if it may make changes).
  Functions anyone may run (see below) get no key name.
- **npm packages:** just import them (`import { nanoid } from "nanoid"`). Bun downloads them on
  first use. Add a `package.json` next to `main.ts` to pin versions.
- **Printing:** `console.log` and `console.error` go to the function's log (the app shows it under
  the code; `function.logs` reads it).

## Reaching your tables
Porchly gives every function two variables:

| Variable | What it is |
|---|---|
| `PORCHLY_URL` | Your API's address on this computer, like `http://127.0.0.1:11911` |
| `PORCHLY_TOKEN` | A key only functions get: it may read and change everything and isn't rate limited. It's new every time Porchly starts and is never stored. |

Use the normal [API](api.md) with it:
```ts
const api = process.env.PORCHLY_URL;
const headers = { Authorization: `Bearer ${process.env.PORCHLY_TOKEN}`, "Content-Type": "application/json" };

export default {
  async fetch(request: Request): Promise<Response> {
    const { order_id } = await request.json();
    const order = await (await fetch(`${api}/api/orders/${order_id}?expand=customer`, { headers })).json();
    await fetch(`${api}/api/orders/${order_id}`, { method: "PATCH", headers, body: JSON.stringify({ invoiced: true }) });
    return Response.json({ sent_to: order.expand.customer.email });
  },
};
```
Change data through the API, never by opening `porchly.db` directly: the API checks values,
keeps links consistent and takes snapshots.

## Variables for other services
Keys for other services (email, payments…) go in the function's **Variables**, one per line
(`MAIL_KEY=abc123`), and the code reads `process.env.MAIL_KEY`. They're stored in
`function.json`. Names starting with `PORCHLY_` are reserved. Functions don't see any other
environment variables from your computer, except basics like `PATH`, `HOME` and proxy settings.

## Who can run it
| Setting | Who | Use |
|---|---|---|
| **Apps with a key** (default) | any valid key, including read-only ones | most functions |
| **Anyone, no key needed** | anybody who knows the address | webhooks from Stripe, GitHub, forms |

With "anyone", check the sender inside your code (for example the webhook's signature). With
the porch light on, "anyone" means anyone on the internet.

## How it runs
Every call starts a fresh Bun process on a free port on `127.0.0.1`, hands it the one request,
streams the answer back and stops it. A function that crashes or hangs only affects its own
call, never Porchly or other functions. Nothing is kept between calls, so store what you need
in a table. A call takes about 5 ms more than a table request (more on first use of an npm
package).

By default there's no time limit. In **Settings** you can have Porchly stop a function that hasn't
answered after 10 seconds, 30 seconds, 1 minute or 5 minutes (`settings.set {"function_timeout": 30}`).
The caller then gets `504 function_timeout`. Porchly sets no memory limit and doesn't cap how many
functions run at once.

## Errors
| Status | Code | When |
|---|---|---|
| 401 | `unauthorized` | The function needs a key and none (or a wrong one) was sent. |
| 404 | `not_found` | No such function, or Functions are off. |
| 413 | `too_large` | The request body is over 10 MB. |
| 500 | `function_failed` | The function threw an error (details in its log). |
| 502 | `function_failed` | The function couldn't start (a syntax error, a bad import, no `export default { fetch }`). The message includes Bun's error. |
| 503 | `functions_unavailable` | Bun couldn't be started. |
| 504 | `function_timeout` | The function didn't answer within the time limit set in Settings. |

## Commands
`functions.status`, `functions.enable`, `functions.disable`, `function.list`, `function.create`
(`template`: `hello` or `tables`), `function.get`, `function.save`, `function.settings`, `function.rename`,
`function.delete`, `function.logs`, `function.call` (tries a function through the running API).
`porchly-cli describe <command>` shows the parameters.
