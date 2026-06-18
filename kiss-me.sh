#!/usr/bin/env bash
# SPDX-License-Identifier: GLWTPL

# KISS mod enabler <kiss-me.sh>
# feqalmatter, 2026

set -Eeuo pipefail
shopt -s nullglob

SCRIPT_NAME="${0##*/}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd -- "$SCRIPT_DIR" || exit

### Parameters ###

GAME_SOURCE="${GAME_SOURCE:-$HOME/Games/Heroic/Cyberpunk 2077}"
LIBRARY="${LIBRARY:-$SCRIPT_DIR/Library}"
DOWNLOADS="${DOWNLOADS:-$SCRIPT_DIR/Downloads}"
OUTPUT="${OUTPUT:-$GAME_SOURCE.modded}"
MANIFEST="${MANIFEST:-$SCRIPT_DIR/.manifest}"

##################

if [[ -t 2 ]]; then
	RED=$'\e[31m'
	YELLOW=$'\e[33m'
	RESET=$'\e[0m'
else
	RED=''
	YELLOW=''
	RESET=''
fi

usage() {
	printf 'usage: %s <command>\n\n' "$SCRIPT_NAME"
	printf 'commands:\n'
	printf '  assemble            build a fresh modded game directory\n'
	printf '  extract-downloads   unpack archives from Downloads into Library\n'
	printf '  enable <mod>        enable a mod package\n'
	printf '  disable <mod>       disable a mod package\n'
	printf '  check-library       check enabled mod directory structure\n'
	printf '  generate-overwrite  pack local game changes as an overwrite mod\n'
	printf '  save-manifest       record the current modded game state\n'
	printf '  diff-manifest       show changes since the saved manifest\n'
	printf '  remove-readmes      remove top-level .txt files from mod packages\n'
	printf '  help                show this help\n\n'
	printf 'environment overrides:\n'
	printf '  GAME_SOURCE=%s\n' "$GAME_SOURCE"
	printf '  LIBRARY=%s\n' "$LIBRARY"
	printf '  DOWNLOADS=%s\n' "$DOWNLOADS"
	printf '  OUTPUT=%s\n' "$OUTPUT"
	printf '  MANIFEST=%s\n' "$MANIFEST"
}

die() {
	printf '%serror:%s %s\n' "$RED" "$RESET" "$*" >&2
	exit 1
}

warn() {
	printf '%swarn:%s %s\n' "$YELLOW" "$RESET" "$*" >&2
}

require_command() {
	local cmd=$1
	command -v "$cmd" >/dev/null || die "missing command: $cmd"
}

require_dir() {
	local dir=$1
	local label=$2

	[[ -d "$dir" ]] || die "missing $label: $dir"
}

require_args() {
	local expected=$1
	local command=$2
	shift 2

	if (($# != expected)); then
		usage >&2
		die "$command expects $expected argument(s)"
	fi
}

enabled_packages() {
	find "$LIBRARY" -maxdepth 1 -mindepth 1 -type d ! -name '*.disabled'
}

mod_name() {
	local name=$1
	name="${name%.disabled}"

	[[ -n "$name" ]] || die "mod name must not be empty"
	[[ "$name" != */* ]] || die "mod name must not contain /"
	[[ "$name" != "." && "$name" != ".." ]] || die "invalid mod name: $name"

	printf '%s\n' "$name"
}

known_package_root() {
	local package=$1
	local root

	for root in archive bin r6 red4ext engine mods; do
		[[ -d "$package/$root" ]] && return 0
	done

	return 1
}

main() {
	if (($# == 0)); then
		usage
		exit 1
	fi

	local command=$1
	shift

	case "$command" in
		assemble) require_args 0 "$command" "$@"; assemble ;;
		extract-downloads) require_args 0 "$command" "$@"; extract_downloads ;;
		enable) require_args 1 "$command" "$@"; enable_mod "$1" ;;
		disable) require_args 1 "$command" "$@"; disable_mod "$1" ;;
		check-library) require_args 0 "$command" "$@"; check_library ;;
		generate-overwrite) require_args 0 "$command" "$@"; generate_overwrite ;;
		save-manifest) require_args 0 "$command" "$@"; save_manifest ;;
		diff-manifest) require_args 0 "$command" "$@"; diff_manifest ;;
		remove-readmes) require_args 0 "$command" "$@"; remove_readmes ;;
		help | -h | --help) require_args 0 "$command" "$@"; usage ;;
		*)
			usage >&2
			die "unknown command: $command"
			;;
	esac
}

extract_archive() {
	local archive=$1
	local outdir=$2
	local ext=${archive##*.}

	ext=${ext,,}

	case "$ext" in
		zip)
			require_command unzip
			unzip -q "$archive" -d "$outdir"
			;;
		7z)
			require_command 7z
			7z x -y -o"$outdir" "$archive" >/dev/null
			;;
		rar)
			require_command unrar
			unrar x -idq -op"$outdir" "$archive"
			;;
		*) die "unsupported archive type: $archive" ;;
	esac
}

extract_downloads() {
	require_dir "$DOWNLOADS" "downloads directory"
	mkdir -p -- "$LIBRARY"

	local archives=("$DOWNLOADS"/*.{zip,ZIP,7z,7Z,rar,RAR})
	local added=()
	local archive filename mod outdir

	if ((${#archives[@]} == 0)); then
		echo "No archives found in $DOWNLOADS."
		return
	fi

	for archive in "${archives[@]}"; do
		filename="$(basename -- "$archive")"
		mod="${filename%.*}"
		outdir="$LIBRARY/$mod"

		if [[ -e "$outdir" || -e "$outdir.disabled" ]]; then
			warn "skipping existing mod: $mod"
			continue
		fi

		mkdir -p -- "$outdir"
		if ! extract_archive "$archive" "$outdir"; then
			rm -rf -- "$outdir"
			die "failed to extract: $archive"
		fi

		added+=("$mod")
	done

	if ((${#added[@]} > 0)); then
		echo "New mods added:"
		local mod_name
		for mod_name in "${added[@]}"; do
			printf '  %s\n' "$mod_name"
		done
	else
		echo "No new mods added."
	fi
}

check_package_structure() {
	[[ -d "$LIBRARY" ]] || return 0

	local error=0
	local package
	for package in "$LIBRARY"/*; do
		[[ -d "$package" ]] || continue

		local name
		name="$(basename -- "$package")"

		[[ "$name" == *.disabled ]] && continue

		if ! known_package_root "$package"; then
			printf '%serror:%s unrecognized mod structure: %s\n' "$RED" "$RESET" "$name" >&2
			error=1
		fi
	done

	return $error
}

check_library() {
	if check_package_structure; then
		echo "Library looks okay."
	else
		die "fix library errors before assembling"
	fi
}

enable_mod() {
	require_dir "$LIBRARY" "library directory"

	local name
	name="$(mod_name "$1")"

	local enabled="$LIBRARY/$name"
	local disabled="$enabled.disabled"

	if [[ -d "$enabled" ]]; then
		echo "Already enabled: $name"
		return
	fi

	[[ ! -e "$enabled" ]] || die "enable target exists and is not a directory: $enabled"
	[[ ! -e "$disabled" || -d "$disabled" ]] || die "disabled mod exists and is not a directory: $disabled"
	[[ -d "$disabled" ]] || die "mod not found: $name"
	mv -- "$disabled" "$enabled"
	echo "Enabled: $name"
}

disable_mod() {
	require_dir "$LIBRARY" "library directory"

	local name
	name="$(mod_name "$1")"

	local enabled="$LIBRARY/$name"
	local disabled="$enabled.disabled"

	if [[ -d "$disabled" ]]; then
		echo "Already disabled: $name"
		return
	fi

	[[ ! -e "$disabled" ]] || die "disable target exists and is not a directory: $disabled"
	[[ ! -e "$enabled" || -d "$enabled" ]] || die "enabled mod exists and is not a directory: $enabled"
	[[ -d "$enabled" ]] || die "mod not found: $name"
	mv -- "$enabled" "$disabled"
	echo "Disabled: $name"
}

remove_readmes() {
	require_dir "$LIBRARY" "library directory"

	local removed=0
	local package readme

	for package in "$LIBRARY"/*; do
		[[ -d "$package" ]] || continue

		for readme in "$package"/*.txt; do
			rm -v -- "$readme"
			removed=1
		done
	done

	((removed == 1)) || echo "No top-level .txt files found in Library."
}

require_safe_output() {
	[[ -n "$OUTPUT" ]] || die "OUTPUT must not be empty"
	[[ "$OUTPUT" != "/" ]] || die "OUTPUT must not be /"

	local output_real source_real
	output_real="$(realpath -m -- "$OUTPUT")"
	source_real="$(realpath -m -- "$GAME_SOURCE")"

	[[ "$output_real" != "$source_real" ]] || die "OUTPUT must differ from GAME_SOURCE"
}

require_cow_output_fs() {
	local output_parent
	output_parent="$(dirname -- "$OUTPUT")"
	require_dir "$output_parent" "output parent directory"

	local fstype
	fstype="$(df --output=fstype "$output_parent" | awk 'NR==2 { print $1 }')"

	case "$fstype" in
		btrfs | zfs | xfs | bcachefs) ;;
		*) die "output parent must be on a reflink-friendly filesystem; found: ${fstype:-unknown}" ;;
	esac
}

assemble() {
	require_dir "$GAME_SOURCE" "game root"
	mkdir -p -- "$LIBRARY"
	require_safe_output
	require_cow_output_fs

	if ! check_package_structure; then
		die "fix library errors before assembling"
	fi

	generate_overwrite --optional

	local assembled="$OUTPUT"

	rm -rf -- "${assembled:?}"
	mkdir -p -- "${assembled}"

	printf 'Copying vanilla game: %s\n' "$GAME_SOURCE"
	cp -a --reflink=auto -- "$GAME_SOURCE"/. "$assembled"/.
	chmod -R u+w -- "$assembled"

	local total
	total="$(enabled_packages | wc -l)"
	local n=1
	local package name

	if ((total == 0)); then
		echo "No enabled mods found; output is a vanilla copy."
		save_manifest
		return
	fi

	for package in "$LIBRARY"/*; do
		[[ -d "$package" ]] || continue

		name="$(basename -- "$package")"
		[[ "$name" == *.disabled ]] && continue

		printf 'Assembling mod %d/%d: %s\n' "$n" "$total" "$name"
		cp -a --reflink=auto -- "$package"/. "$assembled"/.
		((n += 1))
	done

	save_manifest
}

gen_manifest() {
	require_dir "$OUTPUT" "modded game directory"
	find "$OUTPUT" -mindepth 1 -printf '%y\t%P\t%s\t%T@\t%m\t%u\t%g\t%l\n' | LC_ALL=C sort
}

save_manifest() {
	printf 'Recording game state... '
	gen_manifest > "$MANIFEST"
	printf 'saved to %s\n' "$MANIFEST"
}

require_manifest() {
	[[ -f "$MANIFEST" ]] || die "missing manifest: $MANIFEST"
}

manifest_changes() {
	require_manifest
	gen_manifest | LC_ALL=C comm -13 "$MANIFEST" -
}

diff_manifest() {
	manifest_changes
}

overwrite_files() {
	manifest_changes | awk -F '\t' '$1 != "d" { print $2 }'
}

generate_overwrite() {
	local optional=false
	if (($# > 0)); then
		case "$1" in
			--optional) optional=true ;;
			*) die "unknown generate-overwrite option: $1" ;;
		esac
	fi

	if [[ ! -f "$MANIFEST" ]]; then
		[[ "$optional" == true ]] && return 0
		die "missing manifest: $MANIFEST"
	fi

	if [[ ! -d "$OUTPUT" ]]; then
		if [[ "$optional" == true ]]; then
			warn "manifest exists but modded game directory is missing; skipping overwrite generation"
			return 0
		fi

		die "missing modded game directory: $OUTPUT"
	fi

	local changed_files=()
	mapfile -t changed_files < <(overwrite_files)

	if ((${#changed_files[@]} == 0)); then
		[[ "$optional" == true ]] || echo "Game folder appears unchanged; nothing to pack."
		return
	fi

	mkdir -p -- "$LIBRARY"

	local overwrite_mod
	overwrite_mod="$LIBRARY/zzzz-overwrite-$(date +%s)"

	printf 'Packing overwrite mod: %s\n' "$overwrite_mod"

	local file target_dir
	for file in "${changed_files[@]}"; do
		target_dir="$(dirname -- "$overwrite_mod/$file")"
		mkdir -p -- "$target_dir"
		cp -a -- "$OUTPUT/$file" "$overwrite_mod/$file"
		printf '  + %s\n' "$file"
	done
}

main "$@"

