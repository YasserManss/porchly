# Backups and undo

Porchly keeps two safety nets, both automatic.

## Undo
Every time rows are changed or deleted, Porchly remembers what they looked like before. That
covers typing in the app, deleting rows, bulk changes, and everything apps, functions and AI
assistants do through the API.

- In the app, deleting rows shows an **Undo** button. **Backups** (top bar) lists recent changes,
  each with **Undo**.
- Undo puts the rows back the way they were: deleted rows return with the same ids, changed rows
  get their old values, and links that a delete cleared point where they did.
- Any recent change can be undone, not only the last one. Rows changed again since then also go
  back to how they were before that change.
- Changes are kept for **7 days** (the last 200).
- Adding rows isn't recorded (nothing is lost by it; delete them to undo).
- When a table's structure changes (a field is added, renamed, retyped or deleted), the changes
  remembered for that table are forgotten, because old values may no longer fit. Structure
  changes that lose data have their own safety net: a snapshot (below).

```sh
porchly-cli run --project my.db --cmd undo.list      # recent changes, newest first
porchly-cli run --project my.db --cmd undo.run       # undo the newest
porchly-cli run --project my.db --cmd 'undo.run={"id": 12}'
```

## Backups
- **Every day** Porchly copies the project into `backups/` next to it and keeps the last **7**
  days. The copy is made when the project opens, and once a day while Porchly stays open.
- **Before a table or a field is deleted, or a field's type is changed**, Porchly saves a
  snapshot (the last 20 are kept, in `<project>.snapshots/`).
- **Back up now** in the Backups window makes one whenever you like.
- Each backup includes the **functions' code and settings** (not downloaded packages).
- A backup is an ordinary Porchly project file: you can copy it elsewhere, or open it with
  `porchly --project backups/porchly-2026-10-11.db` to look inside.

### Going back to a backup
In **Backups**, click **Go back…** next to one. Tables, rows and functions' code become what
they were then.

- **The way things are now is saved first**, as "Before restoring a backup", so you can come
  back to it.
- **Keys and settings stay as they are today.** A key you turned off after the backup stays
  off.
- Undo history is cleared (it described the state that was replaced).

```sh
porchly-cli run --project my.db --cmd backup.list
porchly-cli run --project my.db --cmd 'backup.restore={"name": "porchly-2026-10-11.db"}'
```

## Good to know
- Backups live on the same computer as your data. For protection against a broken disk or a
  lost laptop, copy the `backups` folder somewhere else now and then.
- Projects saved by this version can't be opened by Porchly 0.1.0 (it will say the file is from
  a newer Porchly).
