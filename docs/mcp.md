# AI assistants (MCP)

Porchly speaks [MCP](https://modelcontextprotocol.io), the way AI assistants such as Claude,
Cursor and VS Code plug into other programs. Connect one and you can say "add this to my
orders table" or "which customers in Lisbon haven't had a delivery?" and it does it with your
tables.

## Connecting
In the app: **Share → Connect an AI assistant…** shows the exact lines to paste, with your
address filled in. An assistant needs a key, like any app (**Share → New key**).

**Assistants that connect to an address** (Claude Code, Cursor, VS Code, and others):
```sh
claude mcp add --transport http porchly http://127.0.0.1:11911/mcp --header "Authorization: Bearer YOUR_KEY"
```
```json
{"mcpServers": {"porchly": {"type": "http", "url": "http://127.0.0.1:11911/mcp",
                            "headers": {"Authorization": "Bearer YOUR_KEY"}}}}
```
The API must be on. With the porch light on, the public address works too
(`https://….trycloudflare.com/mcp`), from anywhere.

**Assistants that start a program** (Claude Desktop, and others):
```json
{"mcpServers": {"porchly": {"command": "/path/to/porchly-cli",
                            "args": ["mcp", "--project", "/path/to/Porchly data/porchly.db"]}}}
```
This way needs no key and no running API. It can only read, unless you add `"--write"` after
`"mcp"`.

## What an assistant can do
| Tool | Does | Needs |
|---|---|---|
| `list_tables` | Tables, their fields and row counts | any key |
| `list_rows` | Rows, with conditions, search, sorting, paging | any key |
| `get_row` | One row | any key |
| `add_rows` | Add up to 1,000 rows (all or nothing) | a key that may make changes |
| `change_rows` | Change the rows matching ids or conditions | a key that may make changes |
| `delete_rows` | Delete the rows matching ids or conditions | a key that may make changes |
| `list_changes`, `undo_change` | See and undo recent changes and deletes | a key that may make changes |
| `create_table`, `add_field` | Make tables and fields | that, **and** the setting below |

An assistant is only shown the tools its key allows.

## Safety
- **A key that can only read can only read.** Start with one.
- **Changes can be undone.** Everything an assistant changes or deletes is recorded like any
  other change: **Backups → Recent changes → Undo**, for 7 days. See [backups.md](backups.md).
- **Changing or deleting needs to say which rows.** A request with no ids or conditions is
  refused, so "delete rows" can't mean "all of them" by accident.
- **Making tables and fields is off** until you tick **Settings → Let AI assistants make new
  tables and fields**.
- **No tool can** delete a table or a field, make or read keys, change settings, turn on the
  porch light, run functions, or run SQL.
- **Through the porch light, assistants can only read** unless you ticked "Allow changes through
  the link". The request limit applies as for any app.
- **Rows are data, not instructions.** Porchly tells assistants so, but no AI is immune to being
  misled by text it reads. Be careful giving a key that may make changes to an assistant that
  works with a table strangers can write to (for example through a public function).

## For developers
`POST /mcp` takes one JSON-RPC 2.0 message (or a batch) and answers with JSON; notifications get
`202`. Protocol versions `2025-06-18`, `2025-03-26` and `2024-11-05`. Only tools are offered (no
resources, prompts or server-sent streams), and there is no OAuth: send the key as
`Authorization: Bearer …`. `porchly-cli mcp` speaks the same protocol, one message per line, on
standard input and output.
