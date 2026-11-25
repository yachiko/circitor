//! Golden files (TST-7). Built by T02.

use std::path::{Path, PathBuf};

/// The switch that makes `check` write instead of compare.
const UPDATE_VAR: &str = "CIRCITOR_UPDATE_GOLDEN";

fn update_switch() -> bool {
    std::env::var(UPDATE_VAR).is_ok_and(|v| v == "1")
}

fn from_root(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// Compares `actual` with the golden file at `path` (relative to the package root); with
/// `CIRCITOR_UPDATE_GOLDEN=1` it writes the file instead (TST-7). On a difference it panics with both texts.
pub fn check(path: &str, actual: &str) {
    run(&from_root(path), actual, update_switch(), Mode::Check);
}

/// The same, but it never writes: for files written by hand from the spec (T41), which no run may overwrite.
pub fn compare(path: &str, actual: &str) {
    run(&from_root(path), actual, update_switch(), Mode::Compare);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Writes when the switch is on.
    Check,
    /// Never writes.
    Compare,
}

/// `check` and `compare` with the file and the switch given, so a test needn't touch the environment.
fn run(file: &Path, actual: &str, update: bool, mode: Mode) {
    if update && mode == Mode::Check {
        if let Some(dir) = file.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            panic!(
                "golden file {}: can't create its directory: {e}",
                file.display()
            );
        }
        if let Err(e) = std::fs::write(file, actual) {
            panic!("golden file {}: can't write it: {e}", file.display());
        }
        return;
    }
    let expected = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => panic!(
            "golden file {}: can't read it ({e}); run with {UPDATE_VAR}=1 to write it\n--- actual ---\n{actual}",
            file.display()
        ),
    };
    if expected != actual {
        panic!(
            "golden file {} differs; run with {UPDATE_VAR}=1 to rewrite it, then read the change\n\
             --- expected ---\n{expected}\n--- actual ---\n{actual}\n--- end ---",
            file.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::ShortDir;

    fn panic_text(f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
        let err = std::panic::catch_unwind(f).err()?;
        Some(err.downcast_ref::<String>().cloned().unwrap_or_default())
    }

    #[test]
    fn golden_check() {
        let dir = ShortDir::new();
        let file = dir.path().join("sub/one.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "expected text\n").unwrap();

        // Equal text passes.
        let f = file.clone();
        assert_eq!(
            panic_text(move || run(&f, "expected text\n", false, Mode::Check)),
            None
        );

        // Different text panics with both texts.
        let f = file.clone();
        let msg =
            panic_text(move || run(&f, "actual text\n", false, Mode::Check)).expect("a panic");
        assert!(
            msg.contains("expected text") && msg.contains("actual text"),
            "{msg}"
        );

        // With the switch on, `check` writes the file and nothing panics.
        let fresh = dir.path().join("new/two.txt");
        let f = fresh.clone();
        assert_eq!(
            panic_text(move || run(&f, "written\n", true, Mode::Check)),
            None
        );
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "written\n");

        // With the switch on, `compare` still panics and writes nothing.
        let f = file.clone();
        let msg =
            panic_text(move || run(&f, "other text\n", true, Mode::Compare)).expect("a panic");
        assert!(msg.contains("other text"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "expected text\n");

        // A missing file panics too, and says how to write it.
        let f = dir.path().join("missing.txt");
        let msg = panic_text(move || run(&f, "x", false, Mode::Compare)).expect("a panic");
        assert!(msg.contains(UPDATE_VAR));
    }

    #[test]
    fn golden_paths_are_from_the_package_root() {
        assert_eq!(
            from_root("tests/golden/x.txt"),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/x.txt")
        );
    }
}
