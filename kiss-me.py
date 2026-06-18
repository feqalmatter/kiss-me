#!/usr/bin/env python3
# SPDX-License-Identifier: GLWTPL
"""KISS mod enabler for Linux game installs."""

from __future__ import annotations

import argparse
import configparser
import errno
import fnmatch
import os
import shutil
import stat
import subprocess
import sys
import time
import zipfile
from dataclasses import dataclass
from pathlib import Path

try:
    import fcntl
except ImportError:  # pragma: no cover - Linux is the target platform.
    fcntl = None

try:
    import grp
    import pwd
except ImportError:  # pragma: no cover - POSIX is the target platform.
    grp = None
    pwd = None


EX_USAGE = 69
CONFIG_FILE = "kiss-me.ini"
DISABLED_SUFFIX = ".disabled"
MANIFEST_VERSION = "# kiss-me manifest v1"
ROOT_MARKERS = ("archive", "bin", "r6", "red4ext", "engine", "mods")
ARCHIVE_EXTENSIONS = {".zip", ".7z", ".rar"}
ENV_TO_PATH_KEY = {
    "KISS_ME_GAME_SOURCE": "game_source",
    "KISS_ME_LIBRARY": "library",
    "KISS_ME_DOWNLOADS": "downloads",
    "KISS_ME_OUTPUT": "output",
    "KISS_ME_MANIFEST": "manifest",
}


class KissMeError(RuntimeError):
    """A user-facing failure that should not print a traceback."""


@dataclass(frozen=True)
class Colors:
    enabled: bool

    def wrap(self, code: str, text: str) -> str:
        if not self.enabled:
            return text
        return f"\033[{code}m{text}\033[0m"

    def red(self, text: str) -> str:
        return self.wrap("31", text)

    def green(self, text: str) -> str:
        return self.wrap("32", text)

    def yellow(self, text: str) -> str:
        return self.wrap("33", text)

    def dim(self, text: str) -> str:
        return self.wrap("2", text)


@dataclass(frozen=True)
class Settings:
    script_dir: Path
    config_path: Path
    game_source: Path
    library: Path
    downloads: Path
    output: Path
    manifest: Path
    dry_run: bool
    verbose: bool
    colors: Colors


def eprint(*parts: object) -> None:
    print(*parts, file=sys.stderr)


def resolve_path(value: str | Path, base: Path) -> Path:
    raw = Path(os.path.expandvars(os.path.expanduser(str(value))))
    if raw.is_absolute():
        return raw
    return base / raw


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(Path.cwd()))
    except ValueError:
        return str(path)


def load_config(config_path: Path, explicit: bool) -> dict[str, str]:
    if not config_path.exists():
        if explicit:
            raise KissMeError(f"Config file does not exist: {config_path}")
        return {}

    parser = configparser.ConfigParser()
    parser.read(config_path)
    if not parser.has_section("paths"):
        return {}
    return {key: value for key, value in parser.items("paths") if value.strip()}


def build_settings(args: argparse.Namespace) -> Settings:
    script_dir = Path(__file__).resolve().parent
    config_path = resolve_path(args.config, Path.cwd()) if args.config else script_dir / CONFIG_FILE

    values: dict[str, str] = {
        "game_source": "~/Games/Heroic/Cyberpunk 2077",
        "library": "Library",
        "downloads": "Downloads",
        "manifest": ".manifest",
    }
    values.update(load_config(config_path, explicit=bool(args.config)))

    for env_name, key in ENV_TO_PATH_KEY.items():
        if os.environ.get(env_name):
            values[key] = os.environ[env_name]

    for key in ("game_source", "library", "downloads", "output", "manifest"):
        cli_value = getattr(args, key, None)
        if cli_value:
            values[key] = str(cli_value)

    game_source = resolve_path(values["game_source"], script_dir)
    output = (
        resolve_path(values["output"], script_dir)
        if values.get("output")
        else Path(f"{game_source}.modded")
    )

    return Settings(
        script_dir=script_dir,
        config_path=config_path,
        game_source=game_source,
        library=resolve_path(values["library"], script_dir),
        downloads=resolve_path(values["downloads"], script_dir),
        output=output,
        manifest=resolve_path(values["manifest"], script_dir),
        dry_run=args.dry_run,
        verbose=args.verbose,
        colors=Colors(sys.stdout.isatty() and not args.no_color and "NO_COLOR" not in os.environ),
    )


def enabled_packages(settings: Settings) -> list[Path]:
    if not settings.library.exists():
        return []
    return [
        path
        for path in sorted(settings.library.iterdir(), key=lambda item: item.name.casefold())
        if path.is_dir() and not path.name.endswith(DISABLED_SUFFIX)
    ]


def all_packages(settings: Settings) -> list[Path]:
    if not settings.library.exists():
        return []
    return [
        path
        for path in sorted(settings.library.iterdir(), key=lambda item: item.name.casefold())
        if path.is_dir()
    ]


def package_enabled_name(package: Path) -> str:
    if package.name.endswith(DISABLED_SUFFIX):
        return package.name[: -len(DISABLED_SUFFIX)]
    return package.name


def has_recognized_structure(package: Path) -> bool:
    return any((package / marker).is_dir() for marker in ROOT_MARKERS)


def command_check_library(settings: Settings, _args: argparse.Namespace | None = None) -> int:
    errors = 0
    for package in enabled_packages(settings):
        if not has_recognized_structure(package):
            print(f"{settings.colors.red('Unrecognized structure')}: {package.name}")
            errors = 1
    if not errors:
        print(settings.colors.green("Library looks OK."))
    return errors


def copy_file_reflink(src: str, dst: str) -> str:
    src_path = Path(src)
    dst_path = Path(dst)
    if dst_path.exists() or dst_path.is_symlink():
        if dst_path.is_dir() and not dst_path.is_symlink():
            shutil.rmtree(dst_path)
        else:
            dst_path.unlink()

    if fcntl is not None:
        try:
            with src_path.open("rb") as src_file, dst_path.open("xb") as dst_file:
                fcntl.ioctl(dst_file.fileno(), 0x40049409, src_file.fileno())  # FICLONE
            shutil.copystat(src_path, dst_path, follow_symlinks=False)
            return str(dst_path)
        except OSError as exc:
            if exc.errno not in {
                errno.EXDEV,
                errno.EOPNOTSUPP,
                errno.ENOTTY,
                errno.EINVAL,
                errno.ENOSYS,
            }:
                raise
            if dst_path.exists() or dst_path.is_symlink():
                dst_path.unlink()

    return shutil.copy2(src_path, dst_path, follow_symlinks=False)


def overlay_tree(source: Path, destination: Path) -> None:
    shutil.copytree(
        source,
        destination,
        copy_function=copy_file_reflink,
        dirs_exist_ok=True,
        symlinks=True,
    )


def make_writable(root: Path) -> None:
    for current, dirnames, filenames in os.walk(root):
        current_path = Path(current)
        current_path.chmod(current_path.stat().st_mode | stat.S_IWUSR)
        for name in dirnames + filenames:
            path = current_path / name
            if path.is_symlink():
                continue
            path.chmod(path.stat().st_mode | stat.S_IWUSR)


def assert_safe_output(settings: Settings) -> None:
    output = settings.output.resolve()
    if output == Path("/"):
        raise KissMeError("Refusing to use / as the assembled output.")
    if output == settings.game_source.resolve():
        raise KissMeError("Output must not be the same directory as the vanilla game source.")
    if output == settings.script_dir.resolve():
        raise KissMeError("Output must not be the tool directory.")


def command_assemble(settings: Settings, args: argparse.Namespace) -> int:
    if not settings.game_source.is_dir():
        raise KissMeError(f"Missing game root: {settings.game_source}")

    if not args.skip_library_check and command_check_library(settings) != 0:
        raise KissMeError("Will not proceed while enabled packages have unrecognized structure.")

    if not args.no_overwrite_capture:
        capture_overwrite(settings, interactive=False)

    assert_safe_output(settings)
    packages = enabled_packages(settings)

    if settings.dry_run:
        print(f"Would remove and recreate: {settings.output}")
        print(f"Would copy vanilla game: {settings.game_source}")
        for package in packages:
            print(f"Would overlay mod: {package.name}")
        print(f"Would save manifest: {settings.manifest}")
        return 0

    if settings.output.exists() or settings.output.is_symlink():
        shutil.rmtree(settings.output)
    settings.output.mkdir(parents=True, exist_ok=True)

    print(f"{settings.colors.green('Copying vanilla game')}: {settings.game_source}")
    overlay_tree(settings.game_source, settings.output)
    make_writable(settings.output)

    total = len(packages)
    for index, package in enumerate(packages, start=1):
        if sys.stdout.isatty():
            print(f"\r\033[KAssembling mod {index}/{total}: {package.name}", end="", flush=True)
        else:
            print(f"Overlaying mod {index}/{total}: {package.name}")
        overlay_tree(package, settings.output)
    if total and sys.stdout.isatty():
        print()

    save_manifest(settings)
    return 0


def user_name(uid: int) -> str:
    if pwd is None:
        return str(uid)
    try:
        return pwd.getpwuid(uid).pw_name
    except KeyError:
        return str(uid)


def group_name(gid: int) -> str:
    if grp is None:
        return str(gid)
    try:
        return grp.getgrgid(gid).gr_name
    except KeyError:
        return str(gid)


def file_type_char(mode: int) -> str:
    if stat.S_ISDIR(mode):
        return "d"
    if stat.S_ISREG(mode):
        return "f"
    if stat.S_ISLNK(mode):
        return "l"
    if stat.S_ISCHR(mode):
        return "c"
    if stat.S_ISBLK(mode):
        return "b"
    if stat.S_ISFIFO(mode):
        return "p"
    if stat.S_ISSOCK(mode):
        return "s"
    return "?"


def iter_tree(root: Path):
    for entry in sorted(os.scandir(root), key=lambda item: item.name):
        path = Path(entry.path)
        yield path
        if entry.is_dir(follow_symlinks=False):
            yield from iter_tree(path)


def generate_manifest(settings: Settings) -> list[str]:
    if not settings.output.is_dir():
        raise KissMeError(f"Missing assembled output: {settings.output}")

    lines: list[str] = []
    for path in iter_tree(settings.output):
        st = path.lstat()
        mode = st.st_mode
        link_target = os.readlink(path) if stat.S_ISLNK(mode) else ""
        rel = path.relative_to(settings.output).as_posix()
        lines.append(
            "\t".join(
                (
                    file_type_char(mode),
                    rel,
                    str(st.st_size),
                    f"{st.st_mtime:.10f}",
                    f"{stat.S_IMODE(mode):o}",
                    user_name(st.st_uid),
                    group_name(st.st_gid),
                    link_target,
                )
            )
        )
    return sorted(lines)


def save_manifest(settings: Settings) -> None:
    if settings.dry_run:
        print(f"Would save manifest: {settings.manifest}")
        return
    settings.manifest.parent.mkdir(parents=True, exist_ok=True)
    settings.manifest.write_text("\n".join(generate_manifest(settings)) + "\n", encoding="utf-8")
    print(f"Recorded game state: {settings.manifest}")


def command_save_manifest(settings: Settings, _args: argparse.Namespace) -> int:
    save_manifest(settings)
    return 0


def read_manifest(path: Path) -> set[str]:
    if not path.exists():
        return set()
    return {
        line
        for line in path.read_text(encoding="utf-8").splitlines()
        if line and not line.startswith("#")
    }


def manifest_diff(settings: Settings) -> list[str]:
    previous = read_manifest(settings.manifest)
    if not previous:
        return []
    return [line for line in generate_manifest(settings) if line not in previous]


def command_diff_manifest(settings: Settings, _args: argparse.Namespace) -> int:
    for line in manifest_diff(settings):
        print(line)
    return 0


def unique_overwrite_dir(settings: Settings, requested_name: str | None) -> Path:
    base_name = requested_name or time.strftime("zzzz-overwrite-%Y%m%d-%H%M%S")
    candidate = settings.library / base_name
    counter = 2
    while candidate.exists() or candidate.with_name(candidate.name + DISABLED_SUFFIX).exists():
        candidate = settings.library / f"{base_name}-{counter}"
        counter += 1
    return candidate


def copy_overwrite_entry(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        if destination.exists() or destination.is_symlink():
            destination.unlink()
        destination.symlink_to(os.readlink(source))
        return
    copy_file_reflink(str(source), str(destination))


def capture_overwrite(
    settings: Settings,
    *,
    interactive: bool,
    requested_name: str | None = None,
) -> int:
    if not settings.manifest.exists():
        if interactive:
            print(f"No manifest found at '{settings.manifest}', have you assembled your mods yet?")
        return 0

    changed: list[Path] = []
    for line in manifest_diff(settings):
        parts = line.split("\t", 2)
        if len(parts) < 2 or parts[0] == "d":
            continue
        source = settings.output / parts[1]
        if source.exists() or source.is_symlink():
            changed.append(source)

    if not changed:
        if interactive:
            print("Game folder appears unchanged, nothing to do.")
        return 0

    overwrite_mod = unique_overwrite_dir(settings, requested_name)
    print(f"Packing new overwrite mod to '{overwrite_mod}'")

    for source in changed:
        relative = source.relative_to(settings.output)
        destination = overwrite_mod / relative
        if settings.dry_run:
            print(f"Would add: {relative.as_posix()}")
            continue
        copy_overwrite_entry(source, destination)
        print(f" + {relative.as_posix()}")
    return 0


def command_gen_overwrite(settings: Settings, args: argparse.Namespace) -> int:
    return capture_overwrite(settings, interactive=True, requested_name=args.name)


def command_init(settings: Settings, args: argparse.Namespace) -> int:
    paths = [settings.library, settings.downloads]
    for path in paths:
        if settings.dry_run:
            print(f"Would create directory: {path}")
        else:
            path.mkdir(parents=True, exist_ok=True)
            print(f"Ready: {path}")

    if args.write_config:
        if settings.config_path.exists() and not args.force:
            raise KissMeError(f"Config already exists: {settings.config_path} (use --force to replace)")
        content = "\n".join(
            (
                "[paths]",
                f"game_source = {settings.game_source}",
                f"library = {settings.library}",
                f"downloads = {settings.downloads}",
                f"output = {settings.output}",
                f"manifest = {settings.manifest}",
                "",
            )
        )
        if settings.dry_run:
            print(f"Would write config: {settings.config_path}")
        else:
            settings.config_path.write_text(content, encoding="utf-8")
            print(f"Wrote config: {settings.config_path}")
    return 0


def external_extract_command(extension: str) -> list[str] | None:
    if extension == ".zip":
        return []
    if extension == ".7z":
        return ["7z"] if shutil.which("7z") else None
    if extension == ".rar":
        if shutil.which("unrar"):
            return ["unrar"]
        if shutil.which("7z"):
            return ["7z"]
        return None
    return None


def safe_extract_zip(archive: Path, outdir: Path) -> None:
    root = outdir.resolve()
    with zipfile.ZipFile(archive) as zip_file:
        for member in zip_file.infolist():
            destination = (outdir / member.filename).resolve()
            if not str(destination).startswith(str(root) + os.sep) and destination != root:
                raise KissMeError(f"Archive contains unsafe path: {member.filename}")
        zip_file.extractall(outdir)


def run_archive_tool(archive: Path, outdir: Path) -> None:
    extension = archive.suffix.casefold()
    if extension == ".zip":
        safe_extract_zip(archive, outdir)
    elif extension == ".7z":
        subprocess.run(["7z", "x", "-y", f"-o{outdir}", str(archive)], check=True)
    elif extension == ".rar" and shutil.which("unrar"):
        subprocess.run(["unrar", "x", "-o+", str(archive), str(outdir) + os.sep], check=True)
    elif extension == ".rar":
        subprocess.run(["7z", "x", "-y", f"-o{outdir}", str(archive)], check=True)
    else:
        raise KissMeError(f"Unsupported archive type: {archive.name}")


def normalize_package(package: Path, settings: Settings) -> None:
    while not has_recognized_structure(package):
        children = [child for child in package.iterdir() if child.name not in (".", "..")]
        if len(children) != 1 or not children[0].is_dir() or children[0].is_symlink():
            return
        wrapper = children[0]
        if not has_recognized_structure(wrapper):
            return

        print(f"{settings.colors.yellow('Flattening wrapper directory')}: {wrapper.name}")
        if settings.dry_run:
            return
        for child in wrapper.iterdir():
            child.rename(package / child.name)
        wrapper.rmdir()


def command_extract_downloads(settings: Settings, _args: argparse.Namespace) -> int:
    if not settings.downloads.exists():
        print(f"Downloads directory does not exist: {settings.downloads}")
        return 0

    archives = [
        path
        for path in sorted(settings.downloads.iterdir(), key=lambda item: item.name.casefold())
        if path.is_file() and path.suffix.casefold() in ARCHIVE_EXTENSIONS
    ]
    if not archives:
        print("Nothing to do.")
        return 0

    missing_tools: set[str] = set()
    for archive in archives:
        command = external_extract_command(archive.suffix.casefold())
        if command is None:
            missing_tools.add("unrar or 7z" if archive.suffix.casefold() == ".rar" else archive.suffix.casefold()[1:])
    if missing_tools:
        for tool in sorted(missing_tools):
            print(f"missing command: {tool}")
        return EX_USAGE

    added: list[str] = []
    if not settings.dry_run:
        settings.library.mkdir(parents=True, exist_ok=True)

    for archive in archives:
        mod_name = archive.name[: -len(archive.suffix)]
        outdir = settings.library / mod_name
        disabled_outdir = settings.library / f"{mod_name}{DISABLED_SUFFIX}"

        if outdir.exists() or disabled_outdir.exists():
            print(f"{settings.colors.yellow('Skipping existing directory')}: {mod_name}")
            continue

        print(f"Extracting: {archive.name} -> {outdir}")
        if not settings.dry_run:
            outdir.mkdir(parents=True)
            try:
                run_archive_tool(archive, outdir)
                normalize_package(outdir, settings)
            except Exception:
                shutil.rmtree(outdir, ignore_errors=True)
                raise
        added.append(mod_name)

    if added:
        print("New mods added:")
        for name in added:
            print(f"  {name}")
    else:
        print("Nothing to do.")
    return 0


def command_remove_readmes(settings: Settings, args: argparse.Namespace) -> int:
    patterns = args.pattern or ["*.txt"]
    removed = 0
    packages = all_packages(settings) if args.include_disabled else enabled_packages(settings)
    for package in packages:
        for child in sorted(package.iterdir(), key=lambda item: item.name.casefold()):
            if not child.is_file():
                continue
            lower_name = child.name.casefold()
            if not any(fnmatch.fnmatchcase(lower_name, pattern.casefold()) for pattern in patterns):
                continue
            if settings.dry_run:
                print(f"Would remove: {child}")
            else:
                child.unlink()
                print(f"Removed: {child}")
            removed += 1
    if not removed:
        print("No readme-ish files found.")
    return 0


def command_list(settings: Settings, _args: argparse.Namespace) -> int:
    packages = all_packages(settings)
    if not packages:
        print(f"No mods found in {settings.library}")
        return 0

    for package in packages:
        disabled = package.name.endswith(DISABLED_SUFFIX)
        name = package_enabled_name(package)
        state = settings.colors.dim("off") if disabled else settings.colors.green("on ")
        structure = ""
        if not disabled and not has_recognized_structure(package):
            structure = f" {settings.colors.red('(unrecognized structure)')}"
        print(f"[{state}] {name}{structure}")
    return 0


def rename_mod(settings: Settings, name: str, enable: bool) -> None:
    clean_name = package_enabled_name(Path(name))
    enabled_path = settings.library / clean_name
    disabled_path = settings.library / f"{clean_name}{DISABLED_SUFFIX}"

    source = disabled_path if enable else enabled_path
    target = enabled_path if enable else disabled_path
    already = enabled_path if enable else disabled_path

    if already.exists() and not source.exists():
        print(f"Already {'enabled' if enable else 'disabled'}: {clean_name}")
        return
    if not source.exists():
        raise KissMeError(f"Unknown mod: {clean_name}")
    if target.exists():
        raise KissMeError(f"Target already exists: {target}")

    action = "Enable" if enable else "Disable"
    if settings.dry_run:
        print(f"Would {action.lower()}: {clean_name}")
    else:
        source.rename(target)
        print(f"{action}d: {clean_name}")


def command_enable(settings: Settings, args: argparse.Namespace) -> int:
    for name in args.mods:
        rename_mod(settings, name, enable=True)
    return 0


def command_disable(settings: Settings, args: argparse.Namespace) -> int:
    for name in args.mods:
        rename_mod(settings, name, enable=False)
    return 0


def command_paths(settings: Settings, _args: argparse.Namespace) -> int:
    rows = (
        ("config", settings.config_path),
        ("game_source", settings.game_source),
        ("library", settings.library),
        ("downloads", settings.downloads),
        ("output", settings.output),
        ("manifest", settings.manifest),
    )
    for key, path in rows:
        exists = settings.colors.green("exists") if path.exists() else settings.colors.dim("missing")
        print(f"{key:12} {path} [{exists}]")
    return 0


def command_doctor(settings: Settings, _args: argparse.Namespace) -> int:
    status = command_paths(settings, _args)
    print()

    for command in ("7z", "unrar"):
        found = shutil.which(command)
        marker = settings.colors.green("ok") if found else settings.colors.yellow("optional")
        print(f"{command:12} {marker}{'' if found else ' (needed for matching archive type)'}")
    print()

    library_status = command_check_library(settings)
    if not settings.game_source.is_dir():
        print(f"{settings.colors.red('Missing game root')}: {settings.game_source}")
        status = 1
    return status or library_status


def add_parser_aliases(
    subparsers: argparse._SubParsersAction,
    name: str,
    *,
    aliases: list[str] | None = None,
    help_text: str,
) -> argparse.ArgumentParser:
    return subparsers.add_parser(name, aliases=aliases or [], help=help_text)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=Path(sys.argv[0]).name,
        description="KISS mod enabler for Linux game installs.",
    )
    parser.add_argument("-n", "--dry-run", action="store_true", help="print planned changes only")
    parser.add_argument("-v", "--verbose", action="store_true", help="show extra details")
    parser.add_argument("--no-color", action="store_true", help="disable ANSI colors")
    parser.add_argument("--config", help=f"path to config file (default: ./{CONFIG_FILE})")
    parser.add_argument("--game-source", help="vanilla game directory")
    parser.add_argument("--library", help="mod library directory")
    parser.add_argument("--downloads", help="archive downloads directory")
    parser.add_argument("--output", help="assembled modded game directory")
    parser.add_argument("--manifest", help="manifest file path")

    subparsers = parser.add_subparsers(dest="command", required=True)

    assemble = add_parser_aliases(subparsers, "assemble", help_text="assemble the modded game tree")
    assemble.add_argument("--skip-library-check", action="store_true", help="assemble even if packages look odd")
    assemble.add_argument("--no-overwrite-capture", action="store_true", help="do not pack changed output files first")
    assemble.set_defaults(func=command_assemble)

    gen_overwrite = add_parser_aliases(
        subparsers,
        "gen-overwrite",
        aliases=["gen_overwrite", "generate-overwrite", "generate_overwrite"],
        help_text="pack changed files from the current output as an overwrite mod",
    )
    gen_overwrite.add_argument("--name", help="overwrite mod directory name")
    gen_overwrite.set_defaults(func=command_gen_overwrite)

    save_manifest_parser = add_parser_aliases(
        subparsers,
        "save-manifest",
        aliases=["save_manifest"],
        help_text="record the current assembled output state",
    )
    save_manifest_parser.set_defaults(func=command_save_manifest)

    diff_manifest_parser = add_parser_aliases(
        subparsers,
        "diff-manifest",
        aliases=["diff_manifest"],
        help_text="print manifest entries that changed since the last save",
    )
    diff_manifest_parser.set_defaults(func=command_diff_manifest)

    extract_parser = add_parser_aliases(
        subparsers,
        "extract-downloads",
        aliases=["extract_downloads", "extract"],
        help_text="extract new zip/7z/rar archives into the library",
    )
    extract_parser.set_defaults(func=command_extract_downloads)

    check_parser = add_parser_aliases(
        subparsers,
        "check-library",
        aliases=["check_library"],
        help_text="check enabled packages for recognizable Cyberpunk roots",
    )
    check_parser.set_defaults(func=command_check_library)

    remove_parser = add_parser_aliases(
        subparsers,
        "remove-readmes",
        aliases=["remove_readmes"],
        help_text="remove root-level readme text files from packages",
    )
    remove_parser.add_argument("--include-disabled", action="store_true", help="also clean disabled packages")
    remove_parser.add_argument(
        "--pattern",
        action="append",
        help="case-insensitive filename pattern to remove (default: *.txt); repeatable",
    )
    remove_parser.set_defaults(func=command_remove_readmes)

    list_parser = add_parser_aliases(subparsers, "list", help_text="list enabled and disabled mods")
    list_parser.set_defaults(func=command_list)

    enable_parser = add_parser_aliases(subparsers, "enable", help_text="enable one or more disabled mods")
    enable_parser.add_argument("mods", nargs="+")
    enable_parser.set_defaults(func=command_enable)

    disable_parser = add_parser_aliases(subparsers, "disable", help_text="disable one or more enabled mods")
    disable_parser.add_argument("mods", nargs="+")
    disable_parser.set_defaults(func=command_disable)

    paths_parser = add_parser_aliases(subparsers, "paths", help_text="show resolved paths")
    paths_parser.set_defaults(func=command_paths)

    doctor_parser = add_parser_aliases(subparsers, "doctor", help_text="check paths, tools, and library structure")
    doctor_parser.set_defaults(func=command_doctor)

    init_parser = add_parser_aliases(subparsers, "init", help_text="create Library/Downloads and optional config")
    init_parser.add_argument("--write-config", action="store_true", help=f"write {CONFIG_FILE}")
    init_parser.add_argument("--force", action="store_true", help="replace an existing config")
    init_parser.set_defaults(func=command_init)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        settings = build_settings(args)
        return int(args.func(settings, args) or 0)
    except KissMeError as exc:
        eprint(f"error: {exc}")
        return EX_USAGE
    except subprocess.CalledProcessError as exc:
        eprint(f"error: command failed with exit {exc.returncode}: {' '.join(exc.cmd)}")
        return exc.returncode or 1
    except KeyboardInterrupt:
        eprint("interrupted")
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
