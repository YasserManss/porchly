# Porchly HTTP API

Every table in a Porchly project is served as a set of REST routes. The API listens on `http://127.0.0.1:11911` by default (`porchly-cli serve --project my.db`). Each project also describes itself in OpenAPI 3.1 at `GET /api/openapi.json`.

## Keys
Every `/api` route needs a key:

```
Authorization: Bearer pk_...
```

Create keys with the `api.key.create` command. A key is shown once, when it is created; Porchly stores only a fingerprint (SHA-256). Keys are **read-only** unless created with `"write": true`. Turn a key off with `api.key.revoke`.

## Routes
| Method | Path | Key | Result |
|---|---|---|---|
| `GET` | `/health` | none | `{"ok": true}` |
| `GET` | `/api` | read | `{"tables": [{"name", "fields": [{"name", "type", "required"}]}]}` |
| `GET` | `/api/openapi.json` | read | OpenAPI 3.1 description of this project |
| `GET` | `/api/{table}` | read | A page of rows (below) |
| `GET` | `/api/{table}/{id}` | read | One row |
| `POST` | `/api/{table}` | write | Creates a row; `201` with the row |
| `PATCH` | `/api/{table}/{id}` | write | Changes the fields sent; returns the row |
| `DELETE` | `/api/{table}/{id}` | write | `204`, no body |
| any | `/api/fn/{name}/…` | read, or none | Runs a function (when Functions are on); see [functions.md](functions.md) |

No table can be called `fn`: that address belongs to functions.

## Rows
Every row has `id`, `created_at` and `updated_at` (set by Porchly, read-only), followed by the table's fields in order:

```json
{"id": 1, "created_at": "2026-01-14T11:35:26.756Z", "updated_at": "2026-01-14T11:35:26.756Z",
 "name": "Pancakes", "minutes": 20, "vegetarian": true}
```

| Field type | JSON | Notes |
|---|---|---|
| `text` | string | up to 1 MB |
| `integer` | number | whole numbers |
| `number` | number | finite |
| `boolean` | `true` / `false` | |
| `datetime` | string | `2026-01-14`, `2026-01-14T14:30:00Z`, `2026-01-14T14:30:00+02:00` |
| `link` | number | the `id` of a row in another table (see [Links between tables](#links-between-tables)) |

Any field may be `null` unless it is required. Unknown fields, and the three system fields, are rejected.

## Listing
`GET /api/{table}` returns:

```json
{"items": [ ...rows... ], "limit": 50, "offset": 0, "total": 3}
```

Query parameters:
- `expand` — link fields whose rows to embed (see below)
- `search` — text to look for in any text field, ignoring case (`?search=ann`); a number also matches that row's `id`. `%` and `_` are ordinary characters.
- `limit` — rows per page (default 50, at most 500)
- `offset` — rows to skip
- `sort` — a field to sort by; `-field` for descending; `link.field` for a field of a linked table. Ties are broken by `id`.
- any field name, or `link.field` — a condition on that field (below). Up to 32 conditions; all must match.

### Conditions
Write `field=value` to match a value exactly, or `field=op.value` to use an operator:

| Operator | Meaning | Example | Works on |
|---|---|---|---|
| *(none)* or `eq` | equals | `?vegetarian=true`, `?minutes=eq.20` | every field |
| `ne` | not equal (rows with no value count as "not equal") | `?vegetarian=ne.true` | every field |
| `lt`, `lte` | less than, less than or equal | `?minutes=lt.30` | numbers, dates, text |
| `gt`, `gte` | greater than, greater than or equal | `?added=gte.2026-10-01` | numbers, dates, text |
| `contains` | text contains, ignoring case | `?name=contains.soup` | text |
| `starts`, `ends` | text starts or ends with, ignoring case | `?name=starts.pan` | text |
| `in` | one of a list (comma-separated, up to 100) | `?serves=in.2,4` | every field |
| `is.empty`, `is.notempty` | has no value / has a value (empty text counts as no value) | `?rating=is.empty` | every field |

Notes:
- Conditions on the same field combine with AND: `?minutes=gte.10&minutes=lt.30`.
- A value that isn't an operator name is matched exactly, so `?name=Dr. Who` works. To match a value that does start with an operator name, put `eq.` in front: `?name=eq.in.time`.
- `%` and `_` in `contains`, `starts` and `ends` are ordinary characters, not wildcards.
- Ignoring case works for English letters (A–Z); other letters must match their case.
- Values are checked against the field type: `?minutes=lt.soon` is a `400` error.
- The `row.list` command takes the same conditions as `"where": {"minutes": "lt.30"}`; `null` means `is.empty`.

## Links between tables
A `link` field points to one row of another table: an order's `customer` holds the `id` of a row in `customers`. Make one with `field.add` (`"type": "link", "links_to": "customers"`) or in the app (Add field → Link to another table).

- **Writing:** send the id: `{"item": "Tea", "customer": 3}`. It must be a row that exists, or `null` for no link.
- **Embedding the linked row:** add `expand` (comma-separated link fields, up to 10). The link stays a number; the row appears under `expand`:

  ```
  GET /api/orders?expand=customer
  {"items": [{"id": 1, "item": "Tea", "customer": 3,
              "expand": {"customer": {"id": 3, "name": "Ann", "city": "Paris", ...}}}], ...}
  ```
  `GET /api/orders/1?expand=customer` works for one row too.
- **Conditions on the linked row:** write `link.field`: `?customer.city=Paris`, `?customer.name=starts.an`. All operators work. Only one link can be followed (`customer.city`, not `customer.company.city`). Rows without a link match none of these conditions, except `is.empty`.
- **Sorting by the linked row:** `?sort=customer.name`, `?sort=-customer.name`.
- **The other direction** ("all orders of customer 3") is a plain condition: `GET /api/orders?customer=3`.
- **Deleting:** deleting a customer clears `customer` in their orders (it becomes `null`); the orders are kept. A table can't be deleted while other tables link to it: delete those link fields first.
- Links can point to the same table (`people.manager` → `people`).

## Errors
Errors are JSON with a stable `code` and a message written for people:

```json
{"error": {"code": "forbidden", "message": "The key \"website\" can only read. Ask the owner for a key that can make changes."}}
```

| Status | `code` | When |
|---|---|---|
| 400 | `invalid` | Bad JSON, wrong value type, missing required field, unknown field, or a condition that doesn't fit the field |
| 401 | `unauthorized` | No key, unknown key, or a key that was turned off |
| 403 | `forbidden` | A read-only key tried to change something |
| 404 | `not_found` | No such table or row |
| 405 | `method_not_allowed` | The route doesn't accept that method |
| 409 | `conflict` | Something with that name already exists, or a table other tables link to was deleted |
| 413 | `too_large` | Body over 1 MB, query string over 8 KB, more than 32 conditions, or more than 100 values in `in` |
| 429 | `rate_limited` | More requests with one key than Settings allow: 20 a second by default (1 to 1000), with bursts of 3 seconds' worth |
| 500 | `internal` / `storage` | A problem inside Porchly; details go to the server log, not the response |

## Limits and protections
- Responses carry `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` (a function may set its own).
- Through the porch light (public link), requests can't make changes unless the owner allows them: `403 forbidden`. See [porch-light.md](porch-light.md).
- No CORS headers yet, so browsers on other sites can't call the API. Allowing chosen sites comes with the desktop app.
- The server log records method, path, status and time. It never records headers, keys or query strings.
