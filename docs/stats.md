# Stats

**Stats** in the top bar shows how much your tables are used and how big they are. The window
keeps itself up to date while it's open.

- **Requests, Rows, Tables, Size on disk**: the headline numbers.
- **Requests in the last hour**: one column per minute; point at a column for its number.
- **Refused** (no key, or a wrong one), **Told to slow down** (more requests than
  [Settings](settings.md) allow) and **Errors** (something went wrong inside Porchly).
- **Changes made**, **Through the porch light** and **By AI assistants**: how many of the
  requests those were.
- **By app**: each key, how many requests it made and when it was last used. A key you don't
  recognise, or one that's much busier than you expect, is worth turning off (Share).
- **By table** and **Functions**: rows, requests and how often each function ran.
- Keys, backups and how many recent changes can still be undone.

## What's counted, and what isn't
Usage is counted in memory **since Porchly opened** and starts again from zero next time. It's
never written to your project and never sent anywhere. Porchly keeps counts only: no addresses
of who called, no contents of requests or rows.

```sh
porchly-cli run --project my.db --cmd stats.get     # the project's numbers; usage is 0 here
```
`stats.get` gives the same numbers to scripts (see the control channel for a running app).
