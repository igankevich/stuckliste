use std::fs::remove_file;
use std::process::Command;
use std::sync::Once;

use arbitrary::Unstructured;
use arbtest::arbtest;
use cpio_test::DirectoryOfFiles;
use tempfile::TempDir;
use test_bin::get_test_bin;

#[test]
fn use_our_mkbom_then_compare_lsbom() {
    compare_lsbom(
        || get_test_bin("mkbom"),
        || get_test_bin("lsbom"),
        || Command::new("lsbom"),
    );
}

#[test]
fn use_their_mkbom_then_compare_lsbom() {
    compare_lsbom(
        || Command::new("mkbom"),
        || Command::new("lsbom"),
        || get_test_bin("lsbom"),
    );
}

#[test]
fn use_our_mkbom_s_then_compare_lsbom() {
    compare_lsbom(
        || {
            let mut command = get_test_bin("mkbom");
            command.arg("-s");
            command
        },
        || get_test_bin("lsbom"),
        || Command::new("lsbom"),
    );
}

#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Only MacOS's original `mkbom` has `-s` argument"
)]
#[test]
fn use_their_mkbom_s_then_compare_lsbom() {
    compare_lsbom(
        || {
            let mut command = Command::new("mkbom");
            command.arg("-s");
            command
        },
        || Command::new("lsbom"),
        || get_test_bin("lsbom"),
    );
}

fn compare_lsbom<F1, F2, F3>(mut our_mkbom: F1, mut our_lsbom: F2, mut their_lsbom: F3)
where
    F1: FnMut() -> Command,
    F2: FnMut() -> Command,
    F3: FnMut() -> Command,
{
    do_not_truncate_assertions();
    let workdir = TempDir::new().unwrap();
    let bom = workdir.path().join("test.bom");
    arbtest(|u| {
        let mut our_mkbom = our_mkbom();
        let mut our_lsbom = our_lsbom();
        let mut their_lsbom = their_lsbom();
        let directory = random_directory(u)?;
        remove_file(&bom).ok();
        let status = our_mkbom.arg(directory.path()).arg(&bom).status().unwrap();
        assert!(status.success());
        let our_output = our_lsbom.arg(&bom).output().unwrap();
        assert!(
            our_output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&our_output.stderr)
        );
        let their_output = their_lsbom.arg(&bom).output().unwrap();
        assert!(
            their_output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&their_output.stderr)
        );
        similar_asserts::assert_eq!(
            normalize_output(&our_output.stdout),
            normalize_output(&their_output.stdout),
        );
        Ok(())
    });
}

fn normalize_output(output: &[u8]) -> String {
    let output = String::from_utf8_lossy(output);
    let mut normalized_output = String::with_capacity(output.len());
    for line in output.split('\n') {
        if line.is_empty() {
            continue;
        }
        normalized_output.push_str(line.trim());
        normalized_output.push('\n');
    }
    normalized_output
}

fn random_directory(u: &mut Unstructured<'_>) -> arbitrary::Result<DirectoryOfFiles> {
    use cpio_test::FileType::*;
    DirectoryOfFiles::new(
        &[
            Regular,
            Directory,
            BlockDevice,
            CharDevice,
            Symlink,
            HardLink,
        ],
        true,
        u,
    )
}

fn do_not_truncate_assertions() {
    NO_TRUNCATE.call_once(|| {
        std::env::set_var("SIMILAR_ASSERTS_MAX_STRING_LENGTH", "0");
    });
}

static NO_TRUNCATE: Once = Once::new();
