# Getting Porchly

Porchly is one file you download and open. Nothing is installed, and it never asks for
administrator rights. To remove it, delete it.

| Computer | Download | Then |
|---|---|---|
| Windows | `porchly-windows-x64.zip` | Open the zip, drag `porchly.exe` to a folder (for example Documents), double-click it. |
| Mac (Apple silicon or Intel) | `porchly-macos.zip` | Double-click the zip, drag `Porchly` to a folder, open it (see below). |
| Linux | `porchly-linux-x64` | Make it runnable (`chmod +x porchly-linux-x64`) and open it. |

Downloads are on the project's GitHub page under **Releases**.

## Where your data lives
Next to the app, in a folder called **Porchly data**: your tables, functions, logs and helper
programs. Copy the app and that folder together (to a USB stick, another computer) and
everything comes along.

If the app's folder can't be written to (for example Program Files, or a Mac running the app
straight from Downloads), Porchly uses a **Porchly** folder in your home folder instead. To
choose yourself: `porchly --project /path/to/my.db`.

## The first time you open it
Porchly isn't signed with a paid certificate yet, so your computer asks once:

- **Windows** says "Windows protected your PC". Click **More info**, then **Run anyway**.
- **Mac** says it can't check the app. Right-click (or Control-click) **Porchly**, choose
  **Open**, then **Open** again. Or go to **System Settings → Privacy & Security** and click
  **Open Anyway**.
- **Linux** doesn't ask.

After that, it opens normally.

## Command line
`porchly-cli` (in the same downloads) serves a project's API and runs commands without a
window. See `docs/api.md`.
