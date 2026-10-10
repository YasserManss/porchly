# Control channel

Local tools (scripts, tests, AI agents) can drive the running desktop app. It is **off** unless the app is started with `--control <port>`, it only listens on `127.0.0.1`, and every connection must send a secret token first, so other programs and other people on the same computer can't use it.

```sh
porchly --project my.db --control 7979
# Control channel on 127.0.0.1:7979  token: 3f9c…
```

The token is printed to the terminal only (never to the log). To choose it yourself, set `PORCHLY_CONTROL_TOKEN` (at least 16 characters) before starting the app.

## Messages
Send one JSON object per line; each gets one JSON line back with the same `id`:

```json
{"id": 1, "method": "auth", "params": {"token": "3f9c…"}}
{"id": 1, "result": {"ok": true}}
```

The first message on a connection must be `auth`. Anything else, or a wrong token, gets an `unauthorized` error and the connection is closed. Errors look like `{"id": 7, "error": {"code": "failed", "message": "…"}}`; codes are `unauthorized`, `invalid` (not JSON), `too_large` (a line over 1 MB), `failed` (the request didn't work) and `timeout` (the app didn't answer within 30 seconds).

## Methods
| Method | Params | Result |
|---|---|---|
| `auth` | `token` | `{"ok": true}` |
| `engine.execute` | `command`, `params` | The command's result. Any engine command (`porchly-cli commands` lists them). The window updates afterwards. |
| `ui.state` | — | `project`, `tables`, `selected`, `rows_shown`, `total_rows`, `offset`, `search`, `share_open`, `dialog`, `notice`, `api`, `porch_light`, `functions_on`, `functions`, `open_function` |
| `ui.select` | `table` | Shows that table. |
| `ui.function` | `name` | Shows that function (when Functions are on). |
| `ui.share` | `open?` (default: toggle) | Opens or closes the Share panel. |
| `ui.search` | `text` | Types into the grid's search box. |
| `ui.screenshot` | `path?` | A PNG of the window: written to `path`, or returned as `png_base64`. Also `width`, `height` (in pixels; larger than the window on high-resolution screens). |

## Example (Python)
```python
import json, socket
s = socket.create_connection(("127.0.0.1", 7979))
f = s.makefile("rw")
def call(i, method, params=None):
    f.write(json.dumps({"id": i, "method": method, "params": params or {}}) + "\n"); f.flush()
    return json.loads(f.readline())

call(1, "auth", {"token": "3f9c…"})
call(2, "engine.execute", {"command": "row.insert", "params": {"table": "recipes", "values": {"name": "Soup"}}})
call(3, "ui.screenshot", {"path": "/tmp/porchly.png"})
```
