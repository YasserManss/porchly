# Command line

`porchly-cli` serves a project's API and runs any command the app can, without a window. It
comes in the same downloads as the app.

```sh
# Create a project with a table, add a row, and make two keys (each key is printed once).
porchly-cli run --project recipes.db \
  --cmd 'table.create={"name":"recipes","fields":[{"name":"name","type":"text","required":true},{"name":"minutes","type":"integer"},{"name":"vegetarian","type":"boolean"}]}' \
  --cmd 'row.insert={"table":"recipes","values":{"name":"Pancakes","minutes":20,"vegetarian":true}}' \
  --cmd 'api.key.create={"name":"my website"}' \
  --cmd 'api.key.create={"name":"me","write":true}'

# Serve it on http://127.0.0.1:11911 (the port from Settings, unless --port is given)
porchly-cli serve --project recipes.db
```

From another terminal, with a key printed above:

```sh
curl -H "Authorization: Bearer pk_..." 'localhost:11911/api/recipes?vegetarian=true&sort=-minutes'
curl -X POST -H "Authorization: Bearer pk_..." -d '{"name":"Chili","minutes":45}' localhost:11911/api/recipes
```

- `porchly-cli commands` lists every command; `porchly-cli describe row.list` explains one.
- `--host 0.0.0.0` lets other computers on your network reach `serve` (a key is still needed).
- The API itself is described in [api.md](api.md), and every project describes itself at
  `GET /api/openapi.json`.
