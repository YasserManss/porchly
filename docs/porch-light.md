# The porch light (public link)

Normally only your own computer can reach Porchly's API. Turning on the **porch light** gives it
a public `https://…trycloudflare.com` address that works from anywhere: a phone app, a website,
a friend's computer. No router settings, no administrator rights, no account.

## Turning it on
In the app: **Share → On the internet → Turn on the porch light…**. Porchly explains what it
means first. From the command line:
```sh
porchly-cli run --project my.db --cmd tunnel.start   # then tunnel.status shows the link
```
(`porchly-cli run` exits when its commands are done, which turns the light off again. Keep the
app open, or use it from a script that stays running, for a link that lasts.)

The first time, Porchly downloads Cloudflare's free `cloudflared` program (about 40 MB), checks it
against a checksum built into Porchly and keeps it in `tools/` next to your project. Set
`PORCHLY_CLOUDFLARED=/path/to/cloudflared` to use one you already have.

## What it changes
- **Keys are still needed.** Without a key, nobody gets anything (except functions you set to
  "Anyone, no key needed").
- **Read-only unless you allow changes.** Through the link, apps can only read, even with a key
  that may make changes. Tick **Allow changes through the link** (`tunnel.allow_changes`) to let
  write keys make changes from anywhere. Porchly remembers this choice.
- **Off at every start.** The light is off whenever Porchly starts, and turning the API off turns
  the light off too.
- **Only the API** is behind the link: never your files or anything else on the computer.
- The functions' own key (`PORCHLY_TOKEN`) doesn't work through the link.
- Functions learn whether the caller may make changes from `X-Porchly-Key-Write`, which is
  `false` through a read-only link.

## Good to know
- **The address changes every time** you turn the light on. Apps using the old one stop working.
- **It's a free Cloudflare "quick tunnel"**, meant for trying things out and sharing for a
  while: no uptime guarantee, up to 200 requests at once. A stable address is planned for later.
- Traffic passes through Cloudflare, which handles the https part.
- Some networks (offices, hotels, schools) block it: `cloudflared` needs to connect out on port
  7844. Porchly says so if connecting takes long.
- Its log is `logs/porch-light.log` next to the project.

## Commands
| Command | Does |
|---|---|
| `tunnel.status` | `{state: off|downloading|connecting|on|failed, url, slow, error, allow_changes}` |
| `tunnel.start` | Turns it on (`allow_changes?`). Turns the API on if it's off. |
| `tunnel.stop` | Turns it off; the link stops working at once. |
| `tunnel.allow_changes` | `{"allow": true|false}`, takes effect at once. |
