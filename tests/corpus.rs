//! Checks every workflow under `LOCALCI_CORPUS`, a list of paths split like `PATH`, so
//! private repositories can be covered without their workflows entering this repository.

use localci::corpus::{check_file, workflow_files, Level};

#[test]
fn every_workflow_in_the_corpus_is_understood() {
    let Some(corpus) = std::env::var_os("LOCALCI_CORPUS") else {
        eprintln!("LOCALCI_CORPUS is not set; skipping the corpus check");
        return;
    };
    let files: Vec<_> = std::env::split_paths(&corpus)
        .flat_map(|p| workflow_files(&p))
        .collect();
    assert!(!files.is_empty(), "no workflow files under LOCALCI_CORPUS");
    let errors: Vec<String> = files
        .iter()
        .flat_map(|f| check_file(f))
        .filter(|f| f.level == Level::Error)
        .map(|f| format!("{}: {}", f.file.display(), f.message))
        .collect();
    assert!(
        errors.is_empty(),
        "{} of {} files:\n{}",
        errors.len(),
        files.len(),
        errors.join("\n")
    );
}

#[test]
fn own_workflows_are_understood() {
    let files = workflow_files(env!("CARGO_MANIFEST_DIR").as_ref());
    for finding in files.iter().flat_map(|f| check_file(f)) {
        assert_ne!(finding.level, Level::Error, "{finding:?}");
    }
}
