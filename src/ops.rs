use crate::config::Profile;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const KNOWN_PACKAGE_ROOTS: &[&str] = &["archive", "bin", "r6", "red4ext", "engine", "mods"];

#[derive(Debug, Clone)]
pub struct OperationContext {
    pub profile: Profile,
    runner: CommandRunner,
}

#[derive(Debug, Clone)]
pub struct CommandRunner;

#[derive(Debug, Clone)]
pub struct ModEntry {
    pub name: String,
    pub enabled: bool,
}

pub trait Reporter {
    fn line(&mut self, line: impl AsRef<str>);
    fn warn(&mut self, line: impl AsRef<str>) {
        self.line(format!("warn: {}", line.as_ref()));
    }
}

pub struct ConsoleReporter;

impl Reporter for ConsoleReporter {
    fn line(&mut self, line: impl AsRef<str>) {
        println!("{}", line.as_ref());
    }
}

impl CommandRunner {
    pub fn real() -> Self {
        Self
    }

    fn run(&self, program: &str, args: &[&OsStr]) -> Result<()> {
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("failed to run {program}"))?;
        if !status.success() {
            bail!("{program} exited with {status}");
        }
        Ok(())
    }

    fn output(&self, program: &str, args: &[&OsStr]) -> Result<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {program}"))?;
        if !output.status.success() {
            bail!("{program} exited with {}", output.status);
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl OperationContext {
    pub fn new(profile: Profile, runner: CommandRunner) -> Self {
        Self { profile, runner }
    }
}

pub fn list_mods(ctx: &OperationContext) -> Result<Vec<ModEntry>> {
    let mut mods = Vec::new();
    if !ctx.profile.library.exists() {
        return Ok(mods);
    }

    for entry in fs::read_dir(&ctx.profile.library)
        .with_context(|| format!("failed to read library: {}", ctx.profile.library.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let raw_name = entry.file_name().to_string_lossy().to_string();
        let (name, enabled) = match raw_name.strip_suffix(".disabled") {
            Some(name) => (name.to_string(), false),
            None => (raw_name, true),
        };
        mods.push(ModEntry { name, enabled });
    }

    mods.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(mods)
}

pub fn extract_downloads(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<()> {
    require_dir(&ctx.profile.downloads, "downloads directory")?;
    fs::create_dir_all(&ctx.profile.library).with_context(|| {
        format!(
            "failed to create library: {}",
            ctx.profile.library.display()
        )
    })?;

    let archives = archives_in(&ctx.profile.downloads)?;
    if archives.is_empty() {
        reporter.line(format!(
            "No archives found in {}.",
            ctx.profile.downloads.display()
        ));
        return Ok(());
    }

    let mut added = Vec::new();
    for archive in archives {
        let filename = archive
            .file_name()
            .and_then(OsStr::to_str)
            .context("archive filename is not valid UTF-8")?;
        let mod_name = archive
            .file_stem()
            .and_then(OsStr::to_str)
            .context("archive stem is not valid UTF-8")?;
        let outdir = ctx.profile.library.join(mod_name);

        if outdir.exists()
            || outdir
                .with_file_name(format!("{mod_name}.disabled"))
                .exists()
        {
            reporter.warn(format!("skipping existing mod: {mod_name}"));
            continue;
        }

        fs::create_dir_all(&outdir)
            .with_context(|| format!("failed to create mod directory: {}", outdir.display()))?;

        if let Err(error) = extract_archive(ctx, &archive, &outdir) {
            let _ = fs::remove_dir_all(&outdir);
            return Err(error).with_context(|| format!("failed to extract: {filename}"));
        }

        added.push(mod_name.to_string());
    }

    if added.is_empty() {
        reporter.line("No new mods added.");
    } else {
        reporter.line("New mods added:");
        for name in added {
            reporter.line(format!("  {name}"));
        }
    }

    Ok(())
}

pub fn enable_mod(ctx: &OperationContext, name: &str, reporter: &mut impl Reporter) -> Result<()> {
    require_dir(&ctx.profile.library, "library directory")?;
    let name = mod_name(name)?;
    let enabled = ctx.profile.library.join(&name);
    let disabled = ctx.profile.library.join(format!("{name}.disabled"));

    if enabled.is_dir() {
        reporter.line(format!("Already enabled: {name}"));
        return Ok(());
    }

    if enabled.exists() {
        bail!(
            "enable target exists and is not a directory: {}",
            enabled.display()
        );
    }
    if disabled.exists() && !disabled.is_dir() {
        bail!(
            "disabled mod exists and is not a directory: {}",
            disabled.display()
        );
    }
    if !disabled.is_dir() {
        bail!("mod not found: {name}");
    }

    fs::rename(&disabled, &enabled).with_context(|| {
        format!(
            "failed to rename {} to {}",
            disabled.display(),
            enabled.display()
        )
    })?;
    reporter.line(format!("Enabled: {name}"));
    Ok(())
}

pub fn disable_mod(ctx: &OperationContext, name: &str, reporter: &mut impl Reporter) -> Result<()> {
    require_dir(&ctx.profile.library, "library directory")?;
    let name = mod_name(name)?;
    let enabled = ctx.profile.library.join(&name);
    let disabled = ctx.profile.library.join(format!("{name}.disabled"));

    if disabled.is_dir() {
        reporter.line(format!("Already disabled: {name}"));
        return Ok(());
    }

    if disabled.exists() {
        bail!(
            "disable target exists and is not a directory: {}",
            disabled.display()
        );
    }
    if enabled.exists() && !enabled.is_dir() {
        bail!(
            "enabled mod exists and is not a directory: {}",
            enabled.display()
        );
    }
    if !enabled.is_dir() {
        bail!("mod not found: {name}");
    }

    fs::rename(&enabled, &disabled).with_context(|| {
        format!(
            "failed to rename {} to {}",
            enabled.display(),
            disabled.display()
        )
    })?;
    reporter.line(format!("Disabled: {name}"));
    Ok(())
}

pub fn check_library(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<()> {
    if check_package_structure(ctx, reporter)? {
        reporter.line("Library looks okay.");
        Ok(())
    } else {
        bail!("fix library errors before assembling");
    }
}

pub fn assemble(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<()> {
    require_dir(&ctx.profile.game_source, "game root")?;
    fs::create_dir_all(&ctx.profile.library).with_context(|| {
        format!(
            "failed to create library: {}",
            ctx.profile.library.display()
        )
    })?;
    require_safe_output(ctx)?;
    require_cow_output_fs(ctx)?;

    if !check_package_structure(ctx, reporter)? {
        bail!("fix library errors before assembling");
    }

    generate_overwrite(ctx, true, reporter)?;

    if ctx.profile.output.exists() {
        fs::remove_dir_all(&ctx.profile.output).with_context(|| {
            format!("failed to remove output: {}", ctx.profile.output.display())
        })?;
    }
    fs::create_dir_all(&ctx.profile.output)
        .with_context(|| format!("failed to create output: {}", ctx.profile.output.display()))?;

    reporter.line(format!(
        "Copying vanilla game: {}",
        ctx.profile.game_source.display()
    ));
    copy_dir_contents(ctx, &ctx.profile.game_source, &ctx.profile.output)?;
    ctx.runner.run(
        "chmod",
        &[
            OsStr::new("-R"),
            OsStr::new("u+w"),
            ctx.profile.output.as_os_str(),
        ],
    )?;

    let enabled = enabled_packages(ctx)?;
    if enabled.is_empty() {
        reporter.line("No enabled mods found; output is a vanilla copy.");
        save_manifest(ctx, reporter)?;
        return Ok(());
    }

    let total = enabled.len();
    for (index, package) in enabled.iter().enumerate() {
        let name = package
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>");
        reporter.line(format!("Assembling mod {}/{}: {name}", index + 1, total));
        copy_dir_contents(ctx, package, &ctx.profile.output)?;
    }

    save_manifest(ctx, reporter)?;
    Ok(())
}

pub fn save_manifest(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<()> {
    reporter.line("Recording game state...");
    let manifest = gen_manifest(ctx)?;
    if let Some(parent) = ctx.profile.manifest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create manifest directory: {}", parent.display())
        })?;
    }
    let mut file = File::create(&ctx.profile.manifest).with_context(|| {
        format!(
            "failed to write manifest: {}",
            ctx.profile.manifest.display()
        )
    })?;
    for line in manifest {
        writeln!(file, "{line}")?;
    }
    reporter.line(format!("saved to {}", ctx.profile.manifest.display()));
    Ok(())
}

pub fn manifest_changes(ctx: &OperationContext) -> Result<Vec<String>> {
    require_file(&ctx.profile.manifest, "manifest")?;
    let saved = fs::read_to_string(&ctx.profile.manifest).with_context(|| {
        format!(
            "failed to read manifest: {}",
            ctx.profile.manifest.display()
        )
    })?;
    let saved: BTreeSet<_> = saved.lines().map(ToOwned::to_owned).collect();
    Ok(gen_manifest(ctx)?
        .into_iter()
        .filter(|line| !saved.contains(line))
        .collect())
}

pub fn generate_overwrite(
    ctx: &OperationContext,
    optional: bool,
    reporter: &mut impl Reporter,
) -> Result<()> {
    if !ctx.profile.manifest.exists() {
        if optional {
            return Ok(());
        }
        bail!("missing manifest: {}", ctx.profile.manifest.display());
    }

    if !ctx.profile.output.is_dir() {
        if optional {
            reporter.warn("manifest exists but modded game directory is missing; skipping overwrite generation");
            return Ok(());
        }
        bail!(
            "missing modded game directory: {}",
            ctx.profile.output.display()
        );
    }

    let changed_files = overwrite_files(ctx)?;
    if changed_files.is_empty() {
        if !optional {
            reporter.line("Game folder appears unchanged; nothing to pack.");
        }
        return Ok(());
    }

    fs::create_dir_all(&ctx.profile.library).with_context(|| {
        format!(
            "failed to create library: {}",
            ctx.profile.library.display()
        )
    })?;
    let overwrite_mod = ctx
        .profile
        .library
        .join(format!("zzzz-overwrite-{}", unix_timestamp()?));
    reporter.line(format!(
        "Packing overwrite mod: {}",
        overwrite_mod.display()
    ));

    for file in changed_files {
        let source = ctx.profile.output.join(&file);
        let target = overwrite_mod.join(&file);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }
        ctx.runner.run(
            "cp",
            &[OsStr::new("-a"), source.as_os_str(), target.as_os_str()],
        )?;
        reporter.line(format!("  + {}", file.display()));
    }

    Ok(())
}

pub fn remove_readmes(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<()> {
    require_dir(&ctx.profile.library, "library directory")?;
    let mut removed = false;

    for package in fs::read_dir(&ctx.profile.library)? {
        let package = package?;
        if !package.file_type()?.is_dir() {
            continue;
        }

        for entry in fs::read_dir(package.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                reporter.line(format!("removed {}", path.display()));
                removed = true;
            }
        }
    }

    if !removed {
        reporter.line("No top-level .txt files found in Library.");
    }
    Ok(())
}

fn archives_in(downloads: &Path) -> Result<Vec<PathBuf>> {
    let mut archives = Vec::new();
    for entry in fs::read_dir(downloads)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
            ext.eq_ignore_ascii_case("zip")
                || ext.eq_ignore_ascii_case("7z")
                || ext.eq_ignore_ascii_case("rar")
        }) {
            archives.push(path);
        }
    }
    archives.sort();
    Ok(archives)
}

fn extract_archive(ctx: &OperationContext, archive: &Path, outdir: &Path) -> Result<()> {
    match archive
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("zip") => {
            let file = File::open(archive)
                .with_context(|| format!("failed to open archive: {}", archive.display()))?;
            let mut zip = zip::ZipArchive::new(file)
                .with_context(|| format!("failed to read zip archive: {}", archive.display()))?;
            zip.extract(outdir)
                .with_context(|| format!("failed to extract zip to {}", outdir.display()))?;
            Ok(())
        }
        Some("7z") => {
            let output_arg = format!("-o{}", outdir.display());
            ctx.runner.run(
                "7z",
                &[
                    OsStr::new("x"),
                    OsStr::new("-y"),
                    OsStr::new(&output_arg),
                    archive.as_os_str(),
                ],
            )
        }
        Some("rar") => {
            let output_arg = format!("-op{}", outdir.display());
            ctx.runner.run(
                "unrar",
                &[
                    OsStr::new("x"),
                    OsStr::new("-idq"),
                    OsStr::new(&output_arg),
                    archive.as_os_str(),
                ],
            )
        }
        _ => bail!("unsupported archive type: {}", archive.display()),
    }
}

fn check_package_structure(ctx: &OperationContext, reporter: &mut impl Reporter) -> Result<bool> {
    if !ctx.profile.library.exists() {
        return Ok(true);
    }

    let mut ok = true;
    for entry in fs::read_dir(&ctx.profile.library)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let path = entry.path();
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        if name.ends_with(".disabled") {
            continue;
        }

        if !KNOWN_PACKAGE_ROOTS
            .iter()
            .any(|root| path.join(root).is_dir())
        {
            reporter.line(format!("error: unrecognized mod structure: {name}"));
            ok = false;
        }
    }

    Ok(ok)
}

fn enabled_packages(ctx: &OperationContext) -> Result<Vec<PathBuf>> {
    let mut packages = Vec::new();
    if !ctx.profile.library.exists() {
        return Ok(packages);
    }
    for entry in fs::read_dir(&ctx.profile.library)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        if !name.ends_with(".disabled") {
            packages.push(path);
        }
    }
    packages.sort();
    Ok(packages)
}

fn mod_name(name: &str) -> Result<String> {
    let name = name.strip_suffix(".disabled").unwrap_or(name).trim();
    if name.is_empty() {
        bail!("mod name must not be empty");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("mod name must not contain path separators");
    }
    if name == "." || name == ".." {
        bail!("invalid mod name: {name}");
    }
    Ok(name.to_string())
}

fn require_dir(path: &Path, label: &str) -> Result<()> {
    if !path.is_dir() {
        bail!("missing {label}: {}", path.display());
    }
    Ok(())
}

fn require_file(path: &Path, label: &str) -> Result<()> {
    if !path.is_file() {
        bail!("missing {label}: {}", path.display());
    }
    Ok(())
}

fn require_safe_output(ctx: &OperationContext) -> Result<()> {
    if ctx.profile.output.as_os_str().is_empty() {
        bail!("OUTPUT must not be empty");
    }
    if ctx.profile.output == Path::new("/") {
        bail!("OUTPUT must not be /");
    }

    let output = absolutize(&ctx.profile.output)?;
    let source = absolutize(&ctx.profile.game_source)?;
    if output == source {
        bail!("OUTPUT must differ from GAME_SOURCE");
    }
    Ok(())
}

fn require_cow_output_fs(ctx: &OperationContext) -> Result<()> {
    let parent = ctx
        .profile
        .output
        .parent()
        .context("OUTPUT must have a parent directory")?;
    require_dir(parent, "output parent directory")?;
    let output = ctx
        .runner
        .output("df", &[OsStr::new("--output=fstype"), parent.as_os_str()])?;
    let fstype = output.lines().nth(1).unwrap_or_default().trim();
    match fstype {
        "btrfs" | "zfs" | "xfs" | "bcachefs" => Ok(()),
        _ => bail!(
            "output parent must be on a reflink-friendly filesystem; found: {}",
            if fstype.is_empty() { "unknown" } else { fstype }
        ),
    }
}

fn copy_dir_contents(ctx: &OperationContext, source: &Path, target: &Path) -> Result<()> {
    let source_dot = source.join(".");
    let target_dot = target.join(".");
    ctx.runner.run(
        "cp",
        &[
            OsStr::new("-a"),
            OsStr::new("--reflink=auto"),
            source_dot.as_os_str(),
            target_dot.as_os_str(),
        ],
    )
}

fn gen_manifest(ctx: &OperationContext) -> Result<Vec<String>> {
    require_dir(&ctx.profile.output, "modded game directory")?;
    let mut lines = Vec::new();

    for entry in WalkDir::new(&ctx.profile.output)
        .min_depth(1)
        .follow_links(false)
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path)?;
        let relative = path
            .strip_prefix(&ctx.profile.output)
            .with_context(|| format!("failed to make relative path: {}", path.display()))?;
        let file_type = file_type_char(&metadata);
        let target = if metadata.file_type().is_symlink() {
            fs::read_link(path)
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}.{:09}\t{:o}\t{}\t{}\t{}",
            file_type,
            relative.display(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.mode() & 0o7777,
            metadata.uid(),
            metadata.gid(),
            target
        ));
    }

    lines.sort();
    Ok(lines)
}

fn overwrite_files(ctx: &OperationContext) -> Result<Vec<PathBuf>> {
    Ok(manifest_changes(ctx)?
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let kind = parts.next()?;
            let path = parts.next()?;
            (kind != "d").then(|| PathBuf::from(path))
        })
        .collect())
}

fn file_type_char(metadata: &fs::Metadata) -> char {
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        'd'
    } else if file_type.is_symlink() {
        'l'
    } else if file_type.is_file() {
        'f'
    } else {
        '?'
    }
}

fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn unix_timestamp() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}
