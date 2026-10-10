# Settings

A few plain choices, under **Settings** in the top bar. They're kept with the project and take
effect at once.

| Setting | Choices | Default | Command value |
|---|---|---|---|
| Requests per second, per app | a number from 1 to 1000 | 20 | `requests_per_second` |
| Stop a function that hasn't answered after | 10 s, 30 s, 1 min, 5 min, no limit (shown when Functions are on) | No limit | `function_timeout`: `10`, `30`, `60`, `300`, `0` |
| Port | 1024 to 65535 | 11911 | `port` (a running API moves to it at once) |
| Turn on the API when Porchly opens | on / off | on | `start_api` |

The request limit counts per key and is always on: apps that ask faster get `429 rate_limited`
and should slow down. Functions' own key isn't limited.

```sh
porchly-cli run --project my.db --cmd 'settings.set={"requests_per_second":100,"function_timeout":30}'
porchly-cli run --project my.db --cmd settings.get
```
A bad value changes nothing. `porchly-cli serve` uses the saved port unless `--port` is given.
