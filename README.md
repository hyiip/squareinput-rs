# Square Input

A shape-based Chinese input method typed on the numeric keypad alone, for
Windows (TSF) and macOS (InputMethodKit), written in Rust.

| Path | What |
|---|---|
| `squareinput/` | The platform-independent core: key state machine, tables, homophones, auto-promotion, window layout, data directory |
| `squareinput-tsf/` | Windows text service (DLL) |
| `squareinput-imk/` | macOS input method (.app) |
| `scripts/` | Install and uninstall scripts |
| `tsv/` | The tables it types from: `codes.tsv`, `punct.tsv`, `homophones.tsv` |

## Install on macOS

You need the Xcode command-line tools (`xcode-select --install`) and Rust
(<https://rustup.rs>).

```sh
git clone https://github.com/hyiip/squareinput-rs.git
cd squareinput-rs
scripts/install-macos.sh
```

The script builds a universal app, signs it ad hoc, installs it into
`~/Library/Input Methods` and copies the tables from `tsv/` into
`~/Library/Application Support/squareinput`. It then registers and enables
the input source, so there is no need to log out. On a first install it also
switches to Square Input; later, switch with the input menu in the menu bar.

If the script says it could not register the input source, add it by hand:

1. Log out and log back in.
2. Open System Settings > Keyboard > Input Sources > Edit, press +, choose
   Chinese, Traditional, then Square Input, and add it.

To update, run `git pull` and then `scripts/install-macos.sh` again. If only
the tables changed, `scripts/install-macos.sh --data-only` is enough.

Errors go to `~/Library/Logs/squareinput/squareinput.log`.

## Install on Windows

You need Rust with the MSVC toolchain and Visual Studio Build Tools, plus the
32-bit target: `rustup target add i686-pc-windows-msvc`.

```powershell
scripts\install.ps1
```

This builds the 64-bit and 32-bit DLLs, copies the tables into
`%APPDATA%\squareinput`, registers the text service and adds Square Input to
the Chinese (Hong Kong) keyboards. Registering raises two UAC prompts the
first time. Switch to it with Win+Space.

Later runs update the DLLs in place without UAC. Run `scripts\install.ps1
-Register` only when the registration itself changes, such as the name shown
in the keyboard list. `scripts\uninstall.ps1` removes it.

Errors go to `%LOCALAPPDATA%\squareinput\squareinput.log`.

## Typing

Everything is on the numeric keypad.

- **Codes:** press 1-9 to pick the left feature, then 1-9 to pick a
  component. Then press 0 to complete a single-part code, or 1-9 for the
  right feature of a split code.
- **Candidates:** press 1-9 to commit one. 0 goes to the next page and - to
  the previous page.
- **Cancel:** `.` cancels whatever is being typed.
- **Punctuation:** when nothing is being typed, `.` opens punctuation. 0 also
  opens it until something has been typed; after that, 0 shows the common
  characters.
- **More menu:** on punctuation page 1, key 9 opens it. 2 arms homophones:
  the next character you pick shows its homophones instead. 4-9 open the
  symbol lists.
- **Other keys:** `/` switches between Chinese and plain digits. `*` moves
  the window between the bottom-right and top-right corners.
- **Auto-promotion:** a character picked from page 3 or later moves to page
  2, key 1. Pages 1 and 2 never move. Homophones learn the same way, per
  reading.

## Data

The tables in `tsv/` are built on the machine that keeps the harvested source
tables in `local/`, which is not in the repository. There, `scripts\install.ps1`
rebuilds the data directory with `squareinput-data` and refreshes `tsv/`.
Commit the changes so other machines get them.

Each machine keeps its own auto-promotion log, `promotions.tsv`, in its data
directory. It is created by typing and is never overwritten by an install.

## Development

```sh
cargo test -p squareinput
cargo clippy --workspace --all-targets
```

The tests against the real tables are skipped when `local/` is missing.
`squareinput-tsf` builds only on Windows and `squareinput-imk` only on macOS;
elsewhere they are empty, so the workspace builds on every platform.
