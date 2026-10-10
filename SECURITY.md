# Security

Porchly puts people's data on a network, so security reports are very welcome.

**Please don't open a public issue for a vulnerability.** Report it privately through
[GitHub's security advisories](https://github.com/YasserManss/porchly/security/advisories/new)
for this repository, or by email to [jiss.saudi@gmail.com](mailto:jiss.saudi@gmail.com).
Include what you found, how to reproduce it, and what an attacker could do. You'll get an answer
within a week.

## What's in scope
- The API: authentication, keys, read-only keys, the read-only public link, rate and size limits.
- Anything that lets someone outside the computer reach data or the computer itself.
- Data loss or corruption caused by API requests or imported files.
- The downloads of helper programs (Bun, cloudflared): checksums and where they come from.

## What isn't
- Functions and plugins do whatever their code says: they're the owner's code and aren't
  sandboxed (see [docs/functions.md](docs/functions.md)). A function marked "Anyone, no key
  needed" being callable by anyone is by design.
- Someone who already controls the computer (they can read the data file directly).
- Cloudflare's own service behind the porch light.

Every security fix comes with a regression test in `crates/api/tests/security.rs` or the crate
it touches.
