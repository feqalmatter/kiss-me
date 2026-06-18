import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "kiss-me.py"
SPEC = importlib.util.spec_from_file_location("kiss_me", MODULE_PATH)
kiss_me = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = kiss_me
SPEC.loader.exec_module(kiss_me)


class KissMeTests(unittest.TestCase):
    def make_settings(self, root: Path, *, dry_run: bool = False):
        return kiss_me.Settings(
            script_dir=root,
            config_path=root / "kiss-me.ini",
            game_source=root / "game",
            library=root / "Library",
            downloads=root / "Downloads",
            output=root / "game.modded",
            manifest=root / ".manifest",
            dry_run=dry_run,
            verbose=False,
            colors=kiss_me.Colors(False),
        )

    def test_enable_disable_round_trip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = self.make_settings(root)
            (settings.library / "CoolMod" / "archive").mkdir(parents=True)

            kiss_me.rename_mod(settings, "CoolMod", enable=False)
            self.assertFalse((settings.library / "CoolMod").exists())
            self.assertTrue((settings.library / "CoolMod.disabled").exists())

            kiss_me.rename_mod(settings, "CoolMod", enable=True)
            self.assertTrue((settings.library / "CoolMod").exists())
            self.assertFalse((settings.library / "CoolMod.disabled").exists())

    def test_manifest_diff_reports_changed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = self.make_settings(root)
            settings.output.mkdir()
            changed_file = settings.output / "notes.txt"
            changed_file.write_text("old\n", encoding="utf-8")

            kiss_me.save_manifest(settings)
            changed_file.write_text("new contents\n", encoding="utf-8")

            diff = kiss_me.manifest_diff(settings)
            self.assertTrue(any("\tnotes.txt\t" in line for line in diff))

    def test_zip_extraction_rejects_path_traversal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "bad.zip"
            outdir = root / "out"
            outdir.mkdir()
            with zipfile.ZipFile(archive, "w") as zip_file:
                zip_file.writestr("../evil.txt", "nope")

            with self.assertRaises(kiss_me.KissMeError):
                kiss_me.safe_extract_zip(archive, outdir)

    def test_normalize_package_flattens_single_wrapper(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = self.make_settings(root)
            package = settings.library / "WrappedMod"
            (package / "Wrapper" / "archive" / "pc" / "mod").mkdir(parents=True)

            kiss_me.normalize_package(package, settings)

            self.assertTrue((package / "archive" / "pc" / "mod").is_dir())
            self.assertFalse((package / "Wrapper").exists())


if __name__ == "__main__":
    unittest.main()
