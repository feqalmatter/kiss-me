# kiss-me

KISS mod enabler for Linux game installs.

The original shell prototype has been rewritten as a dependency-free Python CLI:

```sh
./kiss-me.py <command>
```

`./kiss-me.sh` still works as a compatibility launcher.

## Layout

By default, paths are resolved relative to this repository except for the game
install:

```text
~/Games/Heroic/Cyberpunk 2077          vanilla game source
~/Games/Heroic/Cyberpunk 2077.modded  assembled output
./Library/                            extracted mods
./Downloads/                          downloaded archives
./.manifest                           last assembled file manifest
```

Enabled mods are directories under `Library/`. Disabled mods are the same
directories with a `.disabled` suffix.

## Quick start

```sh
./kiss-me.py init
./kiss-me.py extract-downloads
./kiss-me.py check-library
./kiss-me.py assemble
```

Point Steam/Heroic/Lutris at the `.modded` output once assembly completes.

## Commands

The old underscore command names still work, but dashed names are preferred.

```text
assemble           rebuild the modded game tree
gen-overwrite      pack changed output files as a new overwrite mod
save-manifest      record the current assembled output state
diff-manifest      print entries changed since the last manifest save
extract-downloads  extract new .zip/.7z/.rar archives into Library/
check-library      flag enabled packages without Cyberpunk root folders
remove-readmes     remove root-level readme text files from packages
list               show enabled/disabled mods
enable             enable one or more disabled mods
disable            disable one or more enabled mods
paths              show resolved paths
doctor             check paths, optional archive tools, and library structure
init               create Library/ and Downloads/
```

Useful global flags:

```text
-n, --dry-run      print planned changes without mutating files
-v, --verbose      reserved for noisier diagnostics
--no-color         disable ANSI colors
--config PATH      read a config file from PATH
```

## Configuration

You can keep using the defaults, pass path flags on the command line, set
environment variables, or write `kiss-me.ini`.

Precedence is:

1. command-line flags
2. environment variables
3. `kiss-me.ini`
4. built-in defaults

Supported environment variables:

```text
KISS_ME_GAME_SOURCE
KISS_ME_LIBRARY
KISS_ME_DOWNLOADS
KISS_ME_OUTPUT
KISS_ME_MANIFEST
```

Example `kiss-me.ini`:

```ini
[paths]
game_source = ~/Games/Heroic/Cyberpunk 2077
library = Library
downloads = Downloads
output = ~/Games/Heroic/Cyberpunk 2077.modded
manifest = .manifest
```

Or generate one from the currently resolved paths:

```sh
./kiss-me.py init --write-config
```

## Workflow notes

- `assemble` captures changes in the current output as an overwrite mod before
  rebuilding, as long as `.manifest` exists.
- Overwrite mods are additive: removed files and empty directories are not
  tracked.
- Copying attempts Linux reflinks first and falls back to normal copies, so the
  tool is no longer limited to a short filesystem allow-list.
- Zip extraction uses Python's standard library and rejects unsafe paths.
- `.7z` extraction needs `7z`; `.rar` extraction uses `unrar` when available and
  falls back to `7z`.
- If an archive extracts to a single wrapper directory that contains a Cyberpunk
  root folder such as `archive`, `r6`, `red4ext`, or `mods`, that wrapper is
  flattened automatically.

## Examples

```sh
# See what would be rebuilt.
./kiss-me.py --dry-run assemble

# Temporarily disable a mod.
./kiss-me.py disable FancyMod

# Re-enable it later.
./kiss-me.py enable FancyMod

# Keep a manual tweak from the assembled output.
./kiss-me.py gen-overwrite --name zzzz-overwrite-my-tweak

# Use a different game install without editing config.
./kiss-me.py --game-source "$HOME/Games/Cyberpunk 2077" paths
```