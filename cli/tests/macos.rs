use std::fs::remove_file;
use std::process::Command;
use std::sync::Once;

use arbitrary::Unstructured;
use arbtest::arbtest;
use random_dir::Dir;
use random_dir::DirBuilder;
use tempfile::TempDir;
use test_bin::get_test_bin;

#[test]
fn compare_mkbom() {
    compare_mkbom_and_lsbom(
        || get_test_bin("mkbom"),
        || get_test_bin("lsbom"),
        || Command::new("mkbom"),
        || Command::new("lsbom"),
    );
}

#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Only MacOS's original `mkbom` has `-s` argument"
)]
#[test]
fn compare_mkbom_s() {
    compare_mkbom_and_lsbom(
        || {
            let mut command = get_test_bin("mkbom");
            command.arg("-s");
            command
        },
        || get_test_bin("lsbom"),
        || {
            let mut command = Command::new("mkbom");
            command.arg("-s");
            command
        },
        || Command::new("lsbom"),
    );
}

fn compare_mkbom_and_lsbom<F1, F2, F3, F4>(
    mut our_mkbom: F1,
    mut our_lsbom: F2,
    mut their_mkbom: F3,
    mut their_lsbom: F4,
) where
    F1: FnMut() -> Command,
    F2: FnMut() -> Command,
    F3: FnMut() -> Command,
    F4: FnMut() -> Command,
{
    do_not_truncate_assertions();
    let workdir = TempDir::new().unwrap();
    let our_bom = workdir.path().join("our.bom");
    let their_bom = workdir.path().join("their.bom");
    //let our_bom = "/tmp/our.bom";
    //let their_bom = "/tmp/their.bom";
    arbtest(|u| {
        let directory = random_directory(u)?;
        remove_file(&our_bom).ok();
        remove_file(&their_bom).ok();
        let status = our_mkbom()
            .arg(directory.path())
            .arg(&our_bom)
            .status()
            .unwrap();
        assert!(status.success());
        let status = their_mkbom()
            .arg(directory.path())
            .arg(&their_bom)
            .status()
            .unwrap();
        assert!(status.success());
        // our mkbom+lsbom vs. their mkbom+lsbom
        let our_output_1 = our_lsbom().arg(&our_bom).output().unwrap();
        assert!(
            our_output_1.status.success(),
            "our stderr:\n{}",
            String::from_utf8_lossy(&our_output_1.stderr)
        );
        let their_output_1 = their_lsbom().arg(&their_bom).output().unwrap();
        assert!(
            their_output_1.status.success(),
            "their stderr:\n{}",
            String::from_utf8_lossy(&their_output_1.stderr)
        );
        similar_asserts::assert_eq!(
            normalize_output(&our_output_1.stdout),
            normalize_output(&their_output_1.stdout),
            "our stderr:\n{}",
            String::from_utf8_lossy(&our_output_1.stderr)
        );
        // read their bom with our lsbom
        let our_output_2 = our_lsbom().arg(&their_bom).output().unwrap();
        assert!(
            our_output_2.status.success(),
            "our stderr:\n{}",
            String::from_utf8_lossy(&our_output_2.stderr)
        );
        similar_asserts::assert_eq!(
            normalize_output(&our_output_2.stdout),
            normalize_output(&their_output_1.stdout),
            "our stderr:\n{}",
            String::from_utf8_lossy(&our_output_2.stderr)
        );
        // read our bom with their lsbom
        let their_output_2 = their_lsbom().arg(&our_bom).output().unwrap();
        assert!(
            their_output_2.status.success(),
            "their stderr:\n{}",
            String::from_utf8_lossy(&their_output_2.stderr)
        );
        similar_asserts::assert_eq!(
            normalize_output(&our_output_2.stdout),
            normalize_output(&their_output_2.stdout),
            "our stderr:\n{}",
            String::from_utf8_lossy(&our_output_2.stderr)
        );
        Ok(())
    });
}

fn normalize_output(output: &[u8]) -> String {
    let output = String::from_utf8_lossy(output);
    let mut lines = Vec::new();
    for line in output.split('\n') {
        if line.is_empty() {
            continue;
        }
        lines.push(line.trim());
    }
    lines.sort_unstable();
    lines.join("\n")
}

fn random_directory(u: &mut Unstructured<'_>) -> arbitrary::Result<Dir> {
    use random_dir::FileType::*;
    DirBuilder::new()
        .file_types([
            Regular,
            Directory,
            #[cfg(not(target_os = "macos"))]
            BlockDevice,
            #[cfg(not(target_os = "macos"))]
            CharDevice,
            Symlink,
            HardLink,
        ])
        .printable_names(true)
        .create(u)
}

fn do_not_truncate_assertions() {
    NO_TRUNCATE.call_once(|| {
        std::env::set_var("SIMILAR_ASSERTS_MAX_STRING_LENGTH", "0");
    });
}

static NO_TRUNCATE: Once = Once::new();
