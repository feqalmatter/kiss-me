#!/bin/bash
# SPDX-License-Identifier: GLWTPL

# KISS mod enabler <kiss-me.sh>
# feqalmatter, 2026

cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
shopt -s nullglob

### Parameters ###

GAME_SOURCE="$HOME/Games/Heroic/Cyberpunk 2077"
LIBRARY="$PWD/Library"
DOWNLOADS="$PWD/Downloads"
OUTPUT="$GAME_SOURCE.modded"
MANIFEST="$PWD/.manifest"

##################

usage() {
	echo "usage: $(basename "${BASH_SOURCE[0]}") <command>"
	echo
	echo "command is one of:"
	echo "  assemble"
	echo "  generate_overwrite"
	echo "  save_manifest"
	echo "  diff_manifest"
	echo "  extract_downloads"
	echo "  check_library"
	echo "  remove_readmes"
	echo "  help"
}

main() {
	[[ -n "$2" ]] && usage && exit 69

	case "$1" in
		assemble) assemble ;;
		gen_overwrite) gen_overwrite -i ;;
		save_manifest) save_manifest ;;
		diff_manifest) diff_manifest ;;
		extract_downloads) extract ;;
		check_library) check_package_structure ;;
		remove_readmes) remove_readmes ;;
		help) usage ;;
		*) usage; exit 69 ;;
	esac
}

extract() {
	local prereqs=(
		"unrar"
		"unzip"
		"7z"
	)

	local error=false
	for cmd in "${prereqs[@]}"; do
		if ! command -v "$cmd" >/dev/null; then
			echo "missing command: $cmd"
			error=true
		fi
	done
	$error && exit 69

	local i_total=0
	local a_new=()
	for archive in "$DOWNLOADS"/*.{zip,7z,rar}; do
		((i_total++))
		filename="$(basename "$archive")"
		mod="${filename%.*}"
		outdir="$LIBRARY/$mod"

		if [[ -e "$outdir" || -e "$outdir.disabled" ]]; then
			printf '\e[33mSkipping existing directory\e[0m: %s\n' "$mod"
			continue
		fi

		mkdir -p "$outdir"

		case "${archive##*.}" in
			zip) unzip "$archive" -d "$outdir" ;;
			7z) 7z x -o"$outdir" "$archive" ;;
			rar) unrar x -op"$outdir" "$archive" ;;
		esac

		a_new+=("$mod")
	done

	if [[ ${#a_new[@]} -gt 0 ]]; then
		echo "New mods added:"
		for i in "${a_new[@]}"; do
			echo "  $i"
		done
	else
		echo "Nothing to do."
	fi
}

check_package_structure() {
	# dumb heuristic to try and
	# assess if the extracted mod
	# is in a subfolder
	local error=0
	for package in "$LIBRARY"/*; do
		[[ -d "$package" ]] || continue
		local name="$(basename "$package")"

		[[ "$name" == *.disabled ]] && continue
		
		if [[ ! -d "$package/archive" &&
			! -d "$package/bin" &&
			! -d "$package/r6" &&
			! -d "$package/red4ext" &&
			! -d "$package/engine" &&
			! -d "$package/mods" ]]; then
		printf '\e[31mUnrecognized structure\e[0m: %s\n' "$(basename "$package")"
		error=1
		fi
	done
	return $error
}

remove_readmes() {
	for package in "$LIBRARY"/*; do
		rm -v "$package"/*.txt 2>/dev/null
	done
}

assemble() {
	local output_parent
	output_parent="$(dirname "$OUTPUT")"

	local fstype
	fstype="$(df --output=fstype "$output_parent" | awk 'NR==2 { print $1 }')"

	if [[ ! "$fstype" =~ ^(btrfs|zfs|xfs|bcachefs)$ ]]; then
		echo "Unsupported or unknown filesystem: $fstype"
		exit 69
	fi

	if ! check_package_structure; then
		echo "Will not proceed due to errors."
		exit 69
	fi

	if [[ ! -d "$GAME_SOURCE" ]]; then
		printf '\e[31mMissing game root\e[0m: %s\n' "$GAME_SOURCE"
		exit 69
	fi

	gen_overwrite

	local assembled="$OUTPUT"

	rm -rf -- "${assembled:?}"
	mkdir -p -- "${assembled}"

	#printf '\e[32mCopying vanilla game\e[0m: %s\n' "$GAME_SOURCE"
	cp -a --reflink=auto -- "$GAME_SOURCE"/. "$assembled"/.
	chmod -R u+w "$assembled"

	printf 'Assembling mods'
        
	local total="$(find "$LIBRARY" -maxdepth 1 -mindepth 1 -type d ! -name '*.disabled' | wc -l)"
	local n=1

	for package in "$LIBRARY"/*; do
		[[ -d "$package" ]] || continue

		local name="$(basename "$package")"

		if [[ "$name" == *.disabled ]]; then
			#printf '\e[33mDisabled\e[0m:   %q\n' "$name"
			continue
		fi

		#printf '\e[32mOverlaying\e[0m: %s\n' "$name"
  		printf '\r\033[KAssembling mod %d/%d' "$n" "$total"
		cp -a --reflink=auto -- "$package"/. "$assembled"/.
		((n++))
	done
	printf '\n'

	save_manifest
}

gen_manifest() {
	find "$OUTPUT" -mindepth 1 -printf '%y\t%P\t%s\t%T@\t%m\t%u\t%g\t%l\n' | LC_ALL=C sort
}

save_manifest() {
	printf "Recording game state..."
	gen_manifest > "$MANIFEST"
	printf " saved to '%s'\n" "$MANIFEST"
}

diff_manifest() {
	if [[ -f "$MANIFEST" ]]; then
		gen_manifest | LC_ALL=C comm -13 "$MANIFEST" -
	fi
}

gen_overwrite() {
	# author's note:
	#   overwrite mods are only additive.
	#   files removed are not tracked,
	#   and neither are empty directories.

	if [[ ! -f "$MANIFEST" ]]; then
		[[ "$1" == "-i" ]] && echo "No manifest found at '$MANIFEST', have you assembled your mods yet?"
		return
	fi

	local overwrite_mod="$LIBRARY/zzzz-overwrite-$(date +%s)"
	local diff="$(diff_manifest | sed '/^d/d' | awk -F '\t' '{ print $2 }')"
	[[ -z "$diff" && "$1" == "-i" ]] && echo "Game folder appears unchanged, nothing to do." && return
	[[ -z "$diff" ]] && return

	echo "Packing new overwrite mod to '$overwrite_mod'"

	echo "$diff" | while read -r file; do
		install -D "$OUTPUT/$file" "$overwrite_mod/$file"
		echo " + $file"
	done
}

main "$@"

